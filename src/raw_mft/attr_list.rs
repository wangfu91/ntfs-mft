//! One-level `$ATTRIBUTE_LIST` enrichment for raw and batch MFT entries.
//!
//! NTFS may spill important attributes into extension records when the base
//! FILE record runs out of room. This module decides when that extra work is
//! worthwhile, materializes the flat attribute list, loads the referenced
//! extension records, and merges the relevant metadata back into the caller's
//! base entry.

use crate::NtfsName;

use crate::{
    NtfsFileReference,
    errors::MftError,
    raw_mft::{
        entry_build::{
            AttributeListInfo, EntryBuildOptions, RawMftBatchScratch, RawMftEntry, RawMftLink,
        },
        layout::{
            attribute::{
                AttributeListReference, FileNameNamespace, NtfsAttribute, NtfsAttributeType,
                for_each_attribute_checked, parse_attribute_list_references,
            },
            boot::BootSector,
            extent::ExtentMap,
            record::FileRecord,
        },
        reader::{parse_record_at, read_nonresident},
        volume_reader::VolumeReader,
    },
};
use log::{debug, warn};

/// Counters collected while enriching one base record from its `$ATTRIBUTE_LIST`.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct AttrListEnrichStats {
    /// Number of unique extension-record references discovered in the list.
    pub(super) extension_records_referenced: u64,
    /// Number of referenced extension records that loaded and supplied an
    /// exactly matched attribute used for enrichment.
    pub(super) extension_records_loaded: u64,
}

/// The categories of metadata a base record is still missing.
#[derive(Debug, Clone, Copy, Default)]
struct EnrichmentNeeds {
    /// The base record needs a better file name or additional hard-link names.
    file_name: bool,
    /// The base record needs unnamed-data metadata or ADS details.
    data: bool,
    /// The base record needs a missing reparse tag.
    reparse: bool,
}

#[derive(Clone, Copy)]
struct BaseEnrichmentContext {
    record_number: u64,
    file_reference: u64,
    cluster_size: u64,
}

#[derive(Clone, Copy)]
struct ExtensionRecordRequest<'a> {
    record_number: u64,
    references: &'a [AttributeListReference],
    expected_base: u64,
}

impl EnrichmentNeeds {
    /// Whether any enrichment work remains for the current base record.
    fn any(self) -> bool {
        self.file_name || self.data || self.reparse
    }

    /// Whether the current needs set is interested in a given attribute type.
    fn wants_type(self, type_id: u32) -> bool {
        (self.file_name && type_id == NtfsAttributeType::FileName as u32)
            || (self.data && type_id == NtfsAttributeType::Data as u32)
            || (self.reparse && type_id == NtfsAttributeType::ReparsePoint as u32)
    }
}

/// Borrowed view of the link-related name data carried by one entry.
#[derive(Debug, Clone, Copy)]
struct LinkView<'a> {
    /// Existing boxed links, if the entry already materialized them.
    links: &'a [RawMftLink],
    /// Parent file reference paired with the inline file name.
    parent_reference: NtfsFileReference,
    /// Namespace paired with the inline file name.
    namespace: FileNameNamespace,
    /// Inline file name used when the entry has no boxed link list yet.
    file_name: &'a NtfsName,
}

/// Return `true` when a raw entry should consult its `$ATTRIBUTE_LIST`.
pub(super) fn should_enrich_from_attr_list(entry: &RawMftEntry) -> bool {
    raw_entry_enrichment_needs(entry).any()
}

/// Enrich a raw entry from one level of extension records referenced by its
/// `$ATTRIBUTE_LIST`.
pub(super) fn enrich_from_attr_list(
    entry: &mut RawMftEntry,
    attr_list: AttributeListInfo,
    base_record_number: u64,
    reader: &mut VolumeReader,
    boot: &BootSector,
    extent_map: &ExtentMap,
    build_options: EntryBuildOptions,
) -> AttrListEnrichStats {
    let needs = raw_entry_enrichment_needs(entry);
    if !needs.any() {
        return AttrListEnrichStats::default();
    }

    let mut best_score = if entry.file_name.is_empty() {
        -1
    } else {
        entry.namespace.score()
    };

    with_loaded_extension_records(
        attr_list,
        BaseEnrichmentContext {
            record_number: base_record_number,
            file_reference: entry.file_reference.get(),
            cluster_size: boot.cluster_size,
        },
        reader,
        |type_id| needs.wants_type(type_id),
        |reader, request| load_rich_extension(reader, boot, extent_map, request, build_options),
        |ext_entry| {
            if needs.file_name {
                merge_extension_links(entry, &ext_entry);
                if needs.data || needs.reparse {
                    merge_extension_data(entry, &ext_entry, needs);
                }
                let score = ext_entry.namespace.score();
                if score > best_score {
                    best_score = score;
                    entry.namespace = ext_entry.namespace;
                    entry.file_name = ext_entry.file_name;
                    entry.parent_reference = ext_entry.parent_reference;
                    entry.fn_created = ext_entry.fn_created;
                    entry.fn_modified = ext_entry.fn_modified;
                    entry.fn_mft_modified = ext_entry.fn_mft_modified;
                    entry.fn_accessed = ext_entry.fn_accessed;
                }
            } else if needs.data || needs.reparse {
                merge_extension_data(entry, &ext_entry, needs);
            }
        },
    )
}

/// Return `true` when a batch scratch entry should consult its
/// `$ATTRIBUTE_LIST`.
pub(super) fn should_enrich_batch_from_attr_list(entry: &RawMftBatchScratch) -> bool {
    batch_entry_enrichment_needs(entry).any()
}

/// Enrich a batch scratch entry from one level of extension records.
pub(super) fn enrich_batch_from_attr_list(
    entry: &mut RawMftBatchScratch,
    attr_list: AttributeListInfo,
    base_record_number: u64,
    reader: &mut VolumeReader,
    boot: &BootSector,
    extent_map: &ExtentMap,
    collect_dos_file_name_links: bool,
) -> AttrListEnrichStats {
    let base_file_reference = entry.entry.file_reference.get();
    enrich_batch_with_loader(
        entry,
        attr_list,
        BaseEnrichmentContext {
            record_number: base_record_number,
            file_reference: base_file_reference,
            cluster_size: boot.cluster_size,
        },
        reader,
        |reader, request| {
            load_batch_extension(
                reader,
                boot,
                extent_map,
                request,
                collect_dos_file_name_links,
            )
        },
    )
}

/// Enrich a batch scratch entry from captured extension bytes, reading cache
/// misses from the volume. Output selection may exclude required extensions.
/// Cached but malformed records retain the usual best-effort failure behavior.
pub(in crate::raw_mft) fn enrich_batch_from_attr_list_cached(
    entry: &mut RawMftBatchScratch,
    attr_list: AttributeListInfo,
    base_record_number: u64,
    reader: &mut VolumeReader,
    mft: &super::RawMft<'_>,
    collect_dos_file_name_links: bool,
    ext_cache: &std::collections::HashMap<u64, Box<[u8]>>,
) -> AttrListEnrichStats {
    let base_file_reference = entry.entry.file_reference.get();
    enrich_batch_with_loader(
        entry,
        attr_list,
        BaseEnrichmentContext {
            record_number: base_record_number,
            file_reference: base_file_reference,
            cluster_size: mft.boot.cluster_size,
        },
        reader,
        |reader, request| match ext_cache.get(&request.record_number) {
            Some(raw) => Ok(parse_cached_batch_extension(
                raw,
                request,
                collect_dos_file_name_links,
            )),
            None => load_batch_extension(
                reader,
                &mft.boot,
                mft.extent_map.as_ref(),
                request,
                collect_dos_file_name_links,
            ),
        },
    )
}

/// Shared core of batch enrichment, parameterized over how extension records are
/// loaded (from disk or from a cache).
fn enrich_batch_with_loader<L>(
    entry: &mut RawMftBatchScratch,
    attr_list: AttributeListInfo,
    base: BaseEnrichmentContext,
    reader: &mut VolumeReader,
    load_extension: L,
) -> AttrListEnrichStats
where
    L: FnMut(
        &mut VolumeReader,
        ExtensionRecordRequest<'_>,
    ) -> Result<Option<RawMftBatchScratch>, MftError>,
{
    let needs = batch_entry_enrichment_needs(entry);
    if !needs.any() {
        return AttrListEnrichStats::default();
    }

    let mut best_score = if entry.entry.file_name.is_empty() {
        -1
    } else {
        entry.entry.namespace.score()
    };

    with_loaded_extension_records(
        attr_list,
        base,
        reader,
        |type_id| needs.wants_type(type_id),
        load_extension,
        |ext_entry| {
            if needs.file_name {
                merge_batch_extension_links(entry, &ext_entry);
                if needs.data || needs.reparse {
                    merge_batch_extension_data(entry, &ext_entry, needs);
                }
                let score = ext_entry.entry.namespace.score();
                if score > best_score {
                    best_score = score;
                    entry.entry.namespace = ext_entry.entry.namespace;
                    entry.entry.file_name = ext_entry.entry.file_name;
                    entry.entry.parent_reference = ext_entry.entry.parent_reference;
                }
            } else if needs.data || needs.reparse {
                merge_batch_extension_data(entry, &ext_entry, needs);
            }
        },
    )
}

/// Parse a cached extension record and materialize only attributes named by
/// exact `$ATTRIBUTE_LIST` references.
fn parse_cached_batch_extension(
    raw: &[u8],
    request: ExtensionRecordRequest<'_>,
    collect_dos_file_name_links: bool,
) -> Option<RawMftBatchScratch> {
    let mut buf = raw.to_vec();
    if !FileRecord::is_valid(&buf) {
        return None;
    }
    let record = FileRecord::parse(request.record_number, None, &mut buf).ok()?;
    selected_batch_entry(
        &record,
        request.references,
        request.expected_base,
        collect_dos_file_name_links,
    )
}

/// Load one extension record and materialize only attributes selected by
/// exact `$ATTRIBUTE_LIST` references.
fn load_rich_extension(
    reader: &mut VolumeReader,
    boot: &BootSector,
    extent_map: &ExtentMap,
    request: ExtensionRecordRequest<'_>,
    build_options: EntryBuildOptions,
) -> Result<Option<RawMftEntry>, MftError> {
    let Some(record) = parse_record_at(reader, boot, extent_map, request.record_number)? else {
        return Ok(None);
    };
    Ok(
        selected_attributes(&record, request.references, request.expected_base).map(|attributes| {
            RawMftEntry::from_attributes_with_options(&record, attributes, build_options)
        }),
    )
}

/// Load one extension record and build the lean batch projection from only
/// exactly referenced attributes.
fn load_batch_extension(
    reader: &mut VolumeReader,
    boot: &BootSector,
    extent_map: &ExtentMap,
    request: ExtensionRecordRequest<'_>,
    collect_dos_file_name_links: bool,
) -> Result<Option<RawMftBatchScratch>, MftError> {
    let Some(record) = parse_record_at(reader, boot, extent_map, request.record_number)? else {
        return Ok(None);
    };
    Ok(selected_batch_entry(
        &record,
        request.references,
        request.expected_base,
        collect_dos_file_name_links,
    ))
}

fn selected_batch_entry(
    record: &FileRecord<'_>,
    references: &[AttributeListReference],
    expected_base: u64,
    collect_dos_file_name_links: bool,
) -> Option<RawMftBatchScratch> {
    selected_attributes(record, references, expected_base).map(|attributes| {
        RawMftBatchScratch::from_attributes(record, attributes, collect_dos_file_name_links)
    })
}

/// Select each physical attribute at most once after validating the extension
/// record generation, its base reference, and the complete list-entry tuple.
fn selected_attributes<'a>(
    record: &FileRecord<'a>,
    references: &[AttributeListReference],
    expected_base: u64,
) -> Option<Vec<NtfsAttribute<'a>>> {
    if record.base_reference() != expected_base {
        return None;
    }

    let sequence = record.sequence_value();
    let (attributes_offset, used_size) = record.attrs_range();
    let mut selected = Vec::new();
    let _ = for_each_attribute_checked(record.data, attributes_offset, used_size, |attribute| {
        let referenced = references.iter().any(|reference| {
            reference.sequence_number() == sequence && reference.matches(attribute)
        });
        let is_initial_data_extent = attribute.type_id() != NtfsAttributeType::Data as u32
            || attribute
                .nonresident_header()
                .is_none_or(|header| header.lowest_vcn == 0);
        if referenced
            && is_initial_data_extent
            && let Some(selected_attribute) = NtfsAttribute::new(attribute.data())
        {
            selected.push(selected_attribute);
        }
    });
    (!selected.is_empty()).then_some(selected)
}

/// Materialize a flat `$ATTRIBUTE_LIST` payload from either its resident or
/// non-resident form.
fn materialize_attr_list(
    attr_list: AttributeListInfo,
    base_record_number: u64,
    reader: &mut VolumeReader,
    cluster_size: u64,
) -> Option<Vec<u8>> {
    match attr_list {
        AttributeListInfo::Resident(bytes) => Some(bytes),
        AttributeListInfo::NonResident {
            runs_data,
            data_size,
        } => {
            let runs = match crate::raw_mft::layout::data_run::decode_runs(&runs_data) {
                Ok(runs) => runs,
                Err(error) => {
                    warn!(
                        "raw_mft: record {base_record_number}: \
                         failed to decode $ATTRIBUTE_LIST data runs: {error}"
                    );
                    return None;
                }
            };
            match read_nonresident(reader, &runs, cluster_size, data_size) {
                Ok(bytes) => Some(bytes),
                Err(error) => {
                    warn!(
                        "raw_mft: record {base_record_number}: \
                         failed to read non-resident $ATTRIBUTE_LIST: {error}"
                    );
                    None
                }
            }
        }
    }
}

/// Load and visit each referenced extension record that matches the caller's
/// current enrichment needs.
fn with_loaded_extension_records<T, WantsType, LoadExtension, VisitExtension>(
    attr_list: AttributeListInfo,
    base: BaseEnrichmentContext,
    reader: &mut VolumeReader,
    wants_type: WantsType,
    mut load_extension: LoadExtension,
    mut visit_extension: VisitExtension,
) -> AttrListEnrichStats
where
    WantsType: Fn(u32) -> bool,
    LoadExtension:
        FnMut(&mut VolumeReader, ExtensionRecordRequest<'_>) -> Result<Option<T>, MftError>,
    VisitExtension: FnMut(T),
{
    let mut stats = AttrListEnrichStats::default();
    let data = match materialize_attr_list(attr_list, base.record_number, reader, base.cluster_size)
    {
        Some(data) => data,
        None => return stats,
    };

    let reference_groups =
        collect_extension_reference_groups(&data, base.record_number, wants_type);
    stats.extension_records_referenced = reference_groups.len() as u64;

    for (ext_num, references) in reference_groups {
        let request = ExtensionRecordRequest {
            record_number: ext_num,
            references: &references,
            expected_base: base.file_reference,
        };
        let ext_entry = match load_extension(reader, request) {
            Ok(Some(entry)) => {
                stats.extension_records_loaded += 1;
                entry
            }
            Ok(None) => continue,
            Err(error) => {
                debug!(
                    "raw_mft: record {}: \
                     failed to load extension record {ext_num}: {error}",
                    base.record_number
                );
                continue;
            }
        };
        visit_extension(ext_entry);
    }

    stats
}

/// Group exact references by extension-record number while preserving their
/// first-seen order. A malformed tail leaves the valid prefix available.
fn collect_extension_reference_groups<F>(
    data: &[u8],
    base_record_number: u64,
    wants_type: F,
) -> Vec<(u64, Vec<AttributeListReference>)>
where
    F: Fn(u32) -> bool,
{
    let mut groups: Vec<(u64, Vec<AttributeListReference>)> = Vec::new();
    for reference in parse_attribute_list_references(data).entries {
        let record_number = reference.record_number();
        if record_number == base_record_number || !wants_type(reference.type_id) {
            continue;
        }
        match groups
            .iter_mut()
            .find(|(existing, _)| *existing == record_number)
        {
            Some((_, references)) => references.push(reference),
            None => groups.push((record_number, vec![reference])),
        }
    }
    groups
}

/// Compute the metadata categories missing from a rich raw entry.
fn raw_entry_enrichment_needs(entry: &RawMftEntry) -> EnrichmentNeeds {
    EnrichmentNeeds {
        file_name: entry.file_name.is_empty()
            || !matches!(
                entry.namespace,
                FileNameNamespace::Win32 | FileNameNamespace::Win32AndDos
            )
            || entry.hard_link_count > 1,
        data: !entry.is_directory && !entry.has_unnamed_data,
        reparse: entry.is_reparse_point && entry.reparse_tag.is_none(),
    }
}

/// Compute the metadata categories missing from a lean batch scratch entry.
fn batch_entry_enrichment_needs(entry: &RawMftBatchScratch) -> EnrichmentNeeds {
    EnrichmentNeeds {
        file_name: entry.entry.file_name.is_empty()
            || !matches!(
                entry.entry.namespace,
                FileNameNamespace::Win32 | FileNameNamespace::Win32AndDos
            )
            || entry.hard_link_count > 1,
        data: !entry.entry.is_directory && !entry.have_unnamed_data,
        reparse: entry.is_reparse_point && entry.entry.reparse_tag.is_none(),
    }
}

fn merge_extension_data(entry: &mut RawMftEntry, ext_entry: &RawMftEntry, needs: EnrichmentNeeds) {
    if needs.data
        && (ext_entry.real_size > entry.real_size
            || ext_entry.allocated_size > entry.allocated_size)
    {
        entry.real_size = ext_entry.real_size;
        entry.allocated_size = ext_entry.allocated_size;
        entry.has_unnamed_data = ext_entry.has_unnamed_data;
        entry.is_resident = ext_entry.is_resident;
    }
    if needs.data {
        entry.is_sparse |= ext_entry.is_sparse;
        entry.is_compressed |= ext_entry.is_compressed;
        entry.is_encrypted |= ext_entry.is_encrypted;
    }
    if needs.reparse {
        entry.is_reparse_point |= ext_entry.is_reparse_point;
        entry.reparse_tag = ext_entry.reparse_tag;
    }
    if needs.data && !ext_entry.alternate_data_streams.is_empty() {
        let mut ads = entry.alternate_data_streams.to_vec();
        ads.extend_from_slice(&ext_entry.alternate_data_streams);
        entry.alternate_data_streams = ads.into_boxed_slice();
    }
}

fn merge_batch_extension_data(
    entry: &mut RawMftBatchScratch,
    ext_entry: &RawMftBatchScratch,
    needs: EnrichmentNeeds,
) {
    if needs.data
        && (ext_entry.entry.real_size > entry.entry.real_size
            || ext_entry.entry.allocated_size > entry.entry.allocated_size)
    {
        entry.entry.real_size = ext_entry.entry.real_size;
        entry.entry.allocated_size = ext_entry.entry.allocated_size;
        entry.have_unnamed_data = ext_entry.have_unnamed_data;
    }
    if needs.reparse {
        entry.is_reparse_point |= ext_entry.is_reparse_point;
        entry.entry.reparse_tag = ext_entry.entry.reparse_tag;
    }
}

/// Merge file-name links from an extension record into a rich raw entry.
fn merge_extension_links(entry: &mut RawMftEntry, ext_entry: &RawMftEntry) {
    if let Some(links) = merged_links(
        LinkView {
            links: &entry.links,
            parent_reference: entry.parent_reference,
            namespace: entry.namespace,
            file_name: &entry.file_name,
        },
        LinkView {
            links: &ext_entry.links,
            parent_reference: ext_entry.parent_reference,
            namespace: ext_entry.namespace,
            file_name: &ext_entry.file_name,
        },
    ) {
        entry.links = links;
    }
}

/// Merge file-name links from an extension record into a lean batch entry.
fn merge_batch_extension_links(entry: &mut RawMftBatchScratch, ext_entry: &RawMftBatchScratch) {
    if let Some(links) = merged_links(
        LinkView {
            links: &entry.entry.links,
            parent_reference: entry.entry.parent_reference,
            namespace: entry.entry.namespace,
            file_name: &entry.entry.file_name,
        },
        LinkView {
            links: &ext_entry.entry.links,
            parent_reference: ext_entry.entry.parent_reference,
            namespace: ext_entry.entry.namespace,
            file_name: &ext_entry.entry.file_name,
        },
    ) {
        entry.entry.links = links;
    }
}

/// Build the merged set of file-name links contributed by a base record and
/// one extension record.
fn merged_links(base: LinkView<'_>, ext: LinkView<'_>) -> Option<Box<[RawMftLink]>> {
    let has_single_ext_link = ext.links.is_empty() && !ext.file_name.is_empty();
    if !has_single_ext_link && ext.links.is_empty() {
        return None;
    }

    let base_link_count = usize::from(base.links.is_empty() && !base.file_name.is_empty());
    let ext_link_count = if has_single_ext_link {
        1
    } else {
        ext.links.len()
    };
    let mut links = Vec::with_capacity(base.links.len() + base_link_count + ext_link_count);
    links.extend_from_slice(base.links);
    if links.is_empty() && !base.file_name.is_empty() {
        links.push(RawMftLink {
            parent_reference: base.parent_reference,
            namespace: base.namespace,
            file_name: base.file_name.clone(),
        });
    }
    if has_single_ext_link {
        links.push(RawMftLink {
            parent_reference: ext.parent_reference,
            namespace: ext.namespace,
            file_name: ext.file_name.clone(),
        });
    } else {
        links.extend_from_slice(ext.links);
    }
    Some(links.into_boxed_slice())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NtfsName;
    use zerocopy::IntoBytes;

    use crate::raw_mft::layout::attribute::AttributeListEntryHeader;

    fn attr_list_entry(type_id: u32, record_number: u64) -> Vec<u8> {
        let header = AttributeListEntryHeader {
            type_id,
            record_length: std::mem::size_of::<AttributeListEntryHeader>() as u16,
            attribute_name_length: 0,
            attribute_name_offset: 0,
            lowest_vcn: 0,
            file_reference: record_number,
            attribute_id: 0,
        };
        let mut bytes = vec![0u8; std::mem::size_of::<AttributeListEntryHeader>()];
        bytes.copy_from_slice(header.as_bytes());
        bytes
    }

    #[test]
    fn collect_extension_reference_groups_filters_base_and_groups_duplicates() {
        let base_record = 42;
        let mut data = Vec::new();
        data.extend_from_slice(&attr_list_entry(
            NtfsAttributeType::FileName as u32,
            base_record,
        ));
        data.extend_from_slice(&attr_list_entry(NtfsAttributeType::FileName as u32, 100));
        data.extend_from_slice(&attr_list_entry(NtfsAttributeType::Data as u32, 100));
        data.extend_from_slice(&attr_list_entry(NtfsAttributeType::Data as u32, 101));

        let groups = collect_extension_reference_groups(&data, base_record, |type_id| {
            type_id == NtfsAttributeType::FileName as u32
        });

        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].0, 100);
        assert_eq!(groups[0].1.len(), 1);
    }

    #[test]
    fn merged_links_includes_inline_base_and_extension_names() {
        let base_name = NtfsName::from("base.txt");
        let ext_name = NtfsName::from("ext.txt");
        let links = merged_links(
            LinkView {
                links: &[],
                parent_reference: NtfsFileReference::new(5),
                namespace: FileNameNamespace::Win32,
                file_name: &base_name,
            },
            LinkView {
                links: &[],
                parent_reference: NtfsFileReference::new(9),
                namespace: FileNameNamespace::Win32,
                file_name: &ext_name,
            },
        )
        .expect("extension name should produce merged links");

        assert_eq!(links.len(), 2);
        assert_eq!(links[0].parent_reference, NtfsFileReference::new(5));
        assert_eq!(links[1].parent_reference, NtfsFileReference::new(9));
    }

    #[test]
    fn merged_links_preserve_distinct_names_with_same_display() {
        let base = NtfsName::from_utf16(vec![0xd800]);
        let extension = NtfsName::from_utf16(vec![0xd801]);
        assert_eq!(base.to_string_lossy(), extension.to_string_lossy());
        let view = |name| LinkView {
            links: &[],
            parent_reference: NtfsFileReference::new(5),
            namespace: FileNameNamespace::Win32,
            file_name: name,
        };
        let links = merged_links(view(&base), view(&extension)).unwrap();
        assert_eq!(links.len(), 2);
        assert_eq!(links[0].file_name, base);
        assert_eq!(links[1].file_name, extension);
    }

    #[test]
    fn merged_links_returns_none_without_extension_contribution() {
        let base_name = NtfsName::from("base.txt");
        // The extension record carries neither boxed links nor an inline name,
        // so there is nothing to merge.
        let result = merged_links(
            LinkView {
                links: &[],
                parent_reference: NtfsFileReference::new(5),
                namespace: FileNameNamespace::Win32,
                file_name: &base_name,
            },
            LinkView {
                links: &[],
                parent_reference: NtfsFileReference::new(9),
                namespace: FileNameNamespace::Win32,
                file_name: &crate::NtfsName::new(),
            },
        );
        assert!(result.is_none());
    }

    #[test]
    fn collect_extension_reference_groups_selects_by_wanted_type() {
        let base_record = 42;
        let mut data = Vec::new();
        data.extend_from_slice(&attr_list_entry(NtfsAttributeType::FileName as u32, 100));
        data.extend_from_slice(&attr_list_entry(NtfsAttributeType::Data as u32, 100));
        data.extend_from_slice(&attr_list_entry(NtfsAttributeType::Data as u32, 101));

        // Wanting Data pulls in both data-bearing records, deduplicating 100.
        let groups = collect_extension_reference_groups(&data, base_record, |type_id| {
            type_id == NtfsAttributeType::Data as u32
        });
        assert_eq!(
            groups
                .iter()
                .map(|(record_number, _)| *record_number)
                .collect::<Vec<_>>(),
            vec![100, 101]
        );
        assert_eq!(groups[0].1.len(), 1);
        assert_eq!(groups[1].1.len(), 1);
    }

    fn complete_entry() -> RawMftEntry {
        RawMftEntry {
            collection_options: crate::RawMftEntryOptions::default(),
            record_number: 10,
            sequence_number: 1,
            file_reference: NtfsFileReference::new(10),
            parent_reference: NtfsFileReference::new(5),
            base_record_number: 0,
            hard_link_count: 1,
            flags: 0,
            is_used: true,
            is_directory: false,
            is_reparse_point: false,
            reparse_tag: None,
            namespace: FileNameNamespace::Win32,
            file_name: NtfsName::from("file.txt"),
            si_created: crate::Filetime::new(0),
            si_modified: crate::Filetime::new(0),
            si_mft_modified: crate::Filetime::new(0),
            si_accessed: crate::Filetime::new(0),
            si_file_attributes: crate::FileAttributes::empty(),
            fn_created: crate::Filetime::new(0),
            fn_modified: crate::Filetime::new(0),
            fn_mft_modified: crate::Filetime::new(0),
            fn_accessed: crate::Filetime::new(0),
            real_size: 0,
            allocated_size: 0,
            has_unnamed_data: true,
            is_resident: true,
            is_sparse: false,
            is_compressed: false,
            is_encrypted: false,
            alternate_data_streams: Box::default(),
            links: Box::default(),
        }
    }

    #[test]
    fn complete_entry_needs_no_enrichment() {
        assert!(!should_enrich_from_attr_list(&complete_entry()));
    }

    #[test]
    fn multiple_hard_links_need_file_name_enrichment() {
        let mut entry = complete_entry();
        entry.hard_link_count = 2;
        assert!(raw_entry_enrichment_needs(&entry).file_name);
        assert!(should_enrich_from_attr_list(&entry));
    }

    #[test]
    fn non_win32_namespace_needs_file_name_enrichment() {
        let mut entry = complete_entry();
        entry.namespace = FileNameNamespace::Posix;
        assert!(raw_entry_enrichment_needs(&entry).file_name);
    }

    #[test]
    fn empty_name_needs_file_name_enrichment() {
        let mut entry = complete_entry();
        entry.file_name = NtfsName::new();
        assert!(raw_entry_enrichment_needs(&entry).file_name);
    }

    #[test]
    fn file_without_unnamed_data_needs_data_enrichment() {
        let mut entry = complete_entry();
        entry.has_unnamed_data = false;
        assert!(raw_entry_enrichment_needs(&entry).data);
    }

    #[test]
    fn directory_without_data_needs_no_data_enrichment() {
        let mut entry = complete_entry();
        entry.is_directory = true;
        entry.has_unnamed_data = false;
        let needs = raw_entry_enrichment_needs(&entry);
        assert!(!needs.data, "directories never need $DATA enrichment");
    }

    #[test]
    fn reparse_without_tag_needs_reparse_enrichment() {
        let mut entry = complete_entry();
        entry.is_reparse_point = true;
        entry.reparse_tag = None;
        let needs = raw_entry_enrichment_needs(&entry);
        assert!(needs.reparse);
        assert!(!needs.file_name);
        assert!(!needs.data);
    }
}
