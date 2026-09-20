//! Stream-discovery helpers for raw-MFT constructor bootstrapping.

use std::{
    collections::HashSet,
    io::{Read, Seek, SeekFrom},
    sync::Arc,
};

use log::warn;

use crate::{
    errors::MftError,
    raw_mft::{
        DEFAULT_BUFFER_BYTES,
        entry_build::AttributeListInfo,
        inspection::{RecordInspection, inspect_referenced_attributes, inspect_view},
        layout::{
            attribute::{
                AttributeListReference, NtfsAttribute, NtfsAttributeType,
                parse_attribute_list_references,
            },
            boot::BootSector,
            data_run::{DataRun, decode_runs},
            extent::ExtentMap,
            record::{FileRecord, MFT_RECORD_NUMBER},
        },
        reader::{io_err, read_nonresident},
        volume_reader::VolumeReader,
    },
    volume::Volume,
};

/// Shared immutable state discovered while bootstrapping `RawMft`.
pub(super) struct MftBootstrap {
    /// Decoded unnamed `$MFT::$DATA` extent map.
    pub(super) extent_map: Arc<ExtentMap>,
    /// Materialized `$MFT::$BITMAP` contents when available.
    pub(super) bitmap: Arc<[u8]>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StreamExtent {
    lowest_vcn: u64,
    record_number: u64,
    attribute_id: u16,
    runs: Vec<DataRun>,
}

struct MftStreamRuns {
    base_file_reference: u64,
    data_extents: Vec<StreamExtent>,
    data_size: u64,
    bitmap_extents: Vec<StreamExtent>,
    bitmap_size: u64,
    attribute_list: Option<AttributeListInfo>,
}

/// Read FILE record 0, discover its stream runs, and materialize the shared
/// extent map plus bitmap snapshot used by later scans.
pub(super) fn bootstrap_mft_state(
    volume: &Volume,
    boot: &BootSector,
) -> Result<MftBootstrap, MftError> {
    let mut reader = VolumeReader::new(volume, boot.bytes_per_sector as u64, DEFAULT_BUFFER_BYTES)?;
    let mut record0 = read_mft_record_zero(&mut reader, boot)?;
    let mut streams = {
        let record =
            FileRecord::parse(MFT_RECORD_NUMBER, Some(boot.mft_byte_offset), &mut record0)?;
        discover_mft_stream_runs(&record)?
    };
    load_extension_stream_extents(&mut reader, boot, &mut streams)?;

    if streams.data_extents.is_empty() {
        return Err(MftError::MftAttributeMissing("$MFT $DATA"));
    }
    let data_runs = assemble_stream_runs(&streams.data_extents)?;
    let extent_map = Arc::new(checked_mft_extent_map(&data_runs, boot)?);
    // checked_mft_extent_map already proved this multiplication cannot overflow.
    if streams.data_size > extent_map.total_clusters * boot.cluster_size {
        return Err(MftError::InvalidDataRun(
            "MFT extents do not cover the declared data size",
        ));
    }
    let bitmap = load_mft_bitmap(
        &mut reader,
        boot.cluster_size,
        (!streams.bitmap_extents.is_empty())
            .then(|| assemble_stream_runs(&streams.bitmap_extents))
            .transpose()?,
        streams.bitmap_size,
    )?;

    Ok(MftBootstrap { extent_map, bitmap })
}

/// Read FILE record 0 from the raw volume.
fn read_mft_record_zero(reader: &mut VolumeReader, boot: &BootSector) -> Result<Vec<u8>, MftError> {
    let mut record0 = vec![0u8; boot.file_record_size as usize];
    reader
        .seek(SeekFrom::Start(boot.mft_byte_offset))
        .map_err(io_err)?;
    reader.read_exact(&mut record0).map_err(io_err)?;
    Ok(record0)
}

/// Walk FILE record 0 and collect the decoded runlists for `$MFT::$DATA`
/// and `$MFT::$BITMAP`.
fn discover_mft_stream_runs(record: &FileRecord<'_>) -> Result<MftStreamRuns, MftError> {
    let mut streams = MftStreamRuns {
        base_file_reference: record.file_reference(),
        data_extents: Vec::new(),
        data_size: 0,
        bitmap_extents: Vec::new(),
        bitmap_size: 0,
        attribute_list: None,
    };
    capture_stream_attributes(record, &mut streams, true)?;
    Ok(streams)
}

/// Capture `$MFT::$DATA` and `$MFT::$BITMAP` extents from one fixed-up record.
fn capture_stream_attributes(
    record: &FileRecord<'_>,
    streams: &mut MftStreamRuns,
    capture_attribute_list: bool,
) -> Result<(), MftError> {
    let mut inspection = RecordInspection::default();
    let attributes = inspect_view(record, &mut inspection);
    if inspection.has_malformed_attribute() {
        return Err(MftError::InvalidRecordData(
            "malformed MFT bootstrap attributes",
        ));
    }
    for attribute in attributes {
        capture_stream_attribute(record.number, &attribute, streams, capture_attribute_list)?;
    }
    Ok(())
}

fn capture_stream_attribute(
    record_number: u64,
    attr: &NtfsAttribute<'_>,
    streams: &mut MftStreamRuns,
    capture_attribute_list: bool,
) -> Result<(), MftError> {
    let type_id = attr.type_id();
    let unnamed = !attr.has_name();

    if type_id == NtfsAttributeType::Data as u32 && unnamed && attr.is_non_resident() {
        let extent = decode_nonresident_extent(attr, record_number)?;
        if extent.lowest_vcn == 0
            && let Some(header) = attr.nonresident_header()
        {
            streams.data_size = header.data_size;
        }
        insert_extent(&mut streams.data_extents, extent);
    } else if type_id == NtfsAttributeType::Bitmap as u32 && unnamed && attr.is_non_resident() {
        if let Some(header) = attr.nonresident_header() {
            streams.bitmap_size = streams.bitmap_size.max(header.data_size);
        }
        let extent = decode_nonresident_extent(attr, record_number)?;
        insert_extent(&mut streams.bitmap_extents, extent);
    } else if capture_attribute_list
        && type_id == NtfsAttributeType::AttributeList as u32
        && streams.attribute_list.is_none()
    {
        streams.attribute_list = Some(
            capture_attribute_list_info(attr)
                .ok_or(MftError::InvalidRecordData("malformed MFT attribute list"))?,
        );
    }
    Ok(())
}

fn insert_extent(extents: &mut Vec<StreamExtent>, extent: StreamExtent) {
    if !extents.iter().any(|existing| {
        existing.lowest_vcn == extent.lowest_vcn
            && existing.record_number == extent.record_number
            && existing.attribute_id == extent.attribute_id
    }) {
        extents.push(extent);
    }
}

fn capture_attribute_list_info(attr: &NtfsAttribute<'_>) -> Option<AttributeListInfo> {
    if attr.is_non_resident() {
        let header = attr.nonresident_header()?;
        let start = header.data_runs_offset as usize;
        Some(AttributeListInfo::NonResident {
            runs_data: attr.data().get(start..)?.to_vec(),
            data_size: header.data_size,
        })
    } else {
        attr.resident_value()
            .map(|value| AttributeListInfo::Resident(value.to_vec()))
    }
}

/// Load stream extents referenced by record 0's `$ATTRIBUTE_LIST`.
///
/// This is iterative because a newly discovered `$DATA` extent can make a
/// later extension record addressable through the growing partial extent map.
fn load_extension_stream_extents(
    reader: &mut VolumeReader,
    boot: &BootSector,
    streams: &mut MftStreamRuns,
) -> Result<(), MftError> {
    let Some(attribute_list) = streams.attribute_list.take() else {
        return Ok(());
    };
    let data = materialize_attribute_list(reader, boot.cluster_size, attribute_list)?;
    let mut targets = Vec::<AttributeListReference>::new();
    let mut seen = HashSet::new();
    let references = parse_attribute_list_references(&data);
    if !references.complete {
        return Err(MftError::InvalidRecordData("malformed MFT attribute list"));
    }
    for reference in references.entries {
        let type_id = reference.type_id;
        let is_stream = (type_id == NtfsAttributeType::Data as u32
            || type_id == NtfsAttributeType::Bitmap as u32)
            && reference.name.is_empty();
        if !is_stream {
            continue;
        }
        if seen.insert(reference.clone()) {
            targets.push(reference);
        }
    }

    loop {
        let unresolved: Vec<_> = targets
            .iter()
            .filter(|target| !target_is_loaded(streams, target))
            .collect();
        if unresolved.is_empty() {
            return Ok(());
        }

        let prefix_runs = assemble_contiguous_prefix(&streams.data_extents)?;
        if prefix_runs.is_empty() {
            return Err(MftError::MftAttributeMissing("$MFT $DATA VCN 0 extent"));
        }
        let partial_map = checked_mft_extent_map(&prefix_runs, boot)?;
        let before = streams.data_extents.len() + streams.bitmap_extents.len();
        let mut loaded_records = HashSet::new();

        for target in &unresolved {
            let record_number = target.record_number();
            if record_number == MFT_RECORD_NUMBER || !loaded_records.insert(record_number) {
                continue;
            }
            // A growing prefix may not cover the whole extension record yet.
            if partial_map
                .record_location(record_number, &mut Default::default())
                .is_err()
            {
                continue;
            }
            let Some((offset, buf)) =
                reader.record_buffer(&partial_map, record_number, &mut Default::default())?
            else {
                continue;
            };
            if !FileRecord::is_valid(buf) {
                continue;
            }
            let record = FileRecord::parse(record_number, Some(offset), buf)?;
            let selected = inspect_referenced_attributes(
                &record,
                unresolved
                    .iter()
                    .copied()
                    .filter(|reference| reference.record_number() == record_number),
                MFT_RECORD_NUMBER,
                streams.base_file_reference,
            );
            if selected.malformed || !selected.mismatches.is_empty() {
                return Err(MftError::InvalidRecordData(
                    "malformed or mismatched MFT stream extension attributes",
                ));
            }
            for attribute in selected.attributes {
                capture_stream_attribute(record.number, &attribute, streams, false)?;
            }
        }

        let after = streams.data_extents.len() + streams.bitmap_extents.len();
        if after == before {
            return Err(MftError::MftAttributeMissing(
                "$MFT stream extension referenced by $ATTRIBUTE_LIST",
            ));
        }
    }
}

fn target_is_loaded(streams: &MftStreamRuns, target: &AttributeListReference) -> bool {
    if target.record_number() == MFT_RECORD_NUMBER
        && target.file_reference != streams.base_file_reference
    {
        return false;
    }
    let extents = if target.type_id == NtfsAttributeType::Data as u32 {
        &streams.data_extents
    } else {
        &streams.bitmap_extents
    };
    extents.iter().any(|extent| {
        extent.lowest_vcn == target.lowest_vcn as u64
            && extent.record_number == target.record_number()
            && extent.attribute_id == target.id
    })
}

fn materialize_attribute_list(
    reader: &mut VolumeReader,
    cluster_size: u64,
    attribute_list: AttributeListInfo,
) -> Result<Vec<u8>, MftError> {
    match attribute_list {
        AttributeListInfo::Resident(data) => Ok(data),
        AttributeListInfo::NonResident {
            runs_data,
            data_size,
        } => {
            let runs = decode_runs(&runs_data)?;
            read_nonresident(reader, &runs, cluster_size, data_size)
        }
    }
}

/// Assemble extents in lowest-VCN order. Mapping pairs in each attribute
/// extent are relative to that extent's `lowest_vcn`, not implicitly VCN zero.
fn assemble_stream_runs(extents: &[StreamExtent]) -> Result<Vec<DataRun>, MftError> {
    let (runs, complete) = assemble_runs(extents, false)?;
    if !complete {
        return Err(MftError::InvalidDataRun(
            "non-resident stream extents contain a VCN gap or overlap",
        ));
    }
    Ok(runs)
}

fn assemble_contiguous_prefix(extents: &[StreamExtent]) -> Result<Vec<DataRun>, MftError> {
    assemble_runs(extents, true).map(|(runs, _)| runs)
}

fn assemble_runs(
    extents: &[StreamExtent],
    stop_at_gap: bool,
) -> Result<(Vec<DataRun>, bool), MftError> {
    let mut ordered: Vec<_> = extents.iter().collect();
    ordered.sort_by_key(|extent| extent.lowest_vcn);
    let mut expected_vcn = 0u64;
    let mut runs = Vec::new();

    for extent in ordered {
        if extent.lowest_vcn > expected_vcn && stop_at_gap {
            return Ok((runs, false));
        }
        if extent.lowest_vcn < expected_vcn {
            return Err(MftError::InvalidDataRun(
                "non-resident stream extents overlap in VCN space",
            ));
        }
        if extent.lowest_vcn > expected_vcn {
            return Ok((runs, false));
        }
        let clusters = extent.runs.iter().try_fold(0u64, |total, run| {
            let count = match run {
                DataRun::Data { clusters, .. } | DataRun::Sparse { clusters } => *clusters,
            };
            total
                .checked_add(count)
                .ok_or(MftError::InvalidDataRun("stream VCN length overflow"))
        })?;
        expected_vcn = expected_vcn
            .checked_add(clusters)
            .ok_or(MftError::InvalidDataRun("stream VCN end overflow"))?;
        runs.extend_from_slice(&extent.runs);
    }
    Ok((runs, true))
}

fn decode_nonresident_extent(
    attr: &NtfsAttribute<'_>,
    record_number: u64,
) -> Result<StreamExtent, MftError> {
    let header = attr
        .nonresident_header()
        .ok_or(MftError::InvalidRecordData(
            "missing nonresident MFT stream header",
        ))?;
    if header.lowest_vcn < 0 {
        return Err(MftError::InvalidDataRun("negative stream lowest VCN"));
    }
    decode_nonresident_runs(attr).map(|runs| StreamExtent {
        lowest_vcn: header.lowest_vcn as u64,
        record_number,
        attribute_id: attr.header.id,
        runs,
    })
}

/// Load the `$MFT::$BITMAP` stream when it exists.
fn load_mft_bitmap(
    reader: &mut VolumeReader,
    cluster_size: u64,
    bitmap_runs: Option<Vec<DataRun>>,
    bitmap_size: u64,
) -> Result<Arc<[u8]>, MftError> {
    if let Some(bitmap_runs) = bitmap_runs {
        Ok(read_nonresident(reader, &bitmap_runs, cluster_size, bitmap_size)?.into())
    } else {
        warn!("raw_mft: no $MFT $BITMAP; unused-record filtering will be unavailable");
        Ok(Vec::new().into())
    }
}

/// Decode the runlist payload of a non-resident attribute from FILE record 0.
fn decode_nonresident_runs(attr: &NtfsAttribute<'_>) -> Result<Vec<DataRun>, MftError> {
    let header = attr
        .nonresident_header()
        .ok_or(MftError::InvalidRecordData(
            "missing nonresident MFT stream header",
        ))?;
    let runs_offset = header.data_runs_offset as usize;
    let attr_data = attr.data();
    if runs_offset > attr_data.len() {
        return Err(MftError::InvalidDataRun(
            "MFT stream runlist offset out of bounds",
        ));
    }

    decode_runs(&attr_data[runs_offset..])
}

/// Reject impossible capacity before saturating extent arithmetic can turn it
/// into a huge successful scan range. Partial bootstrap maps need the same check.
fn checked_mft_extent_map(runs: &[DataRun], boot: &BootSector) -> Result<ExtentMap, MftError> {
    let clusters = runs.iter().try_fold(0u64, |total, run| {
        let count = match run {
            DataRun::Data { clusters, .. } | DataRun::Sparse { clusters } => *clusters,
        };
        total
            .checked_add(count)
            .ok_or(MftError::InvalidDataRun("MFT cluster count overflow"))
    })?;
    let bytes = clusters
        .checked_mul(boot.cluster_size)
        .ok_or(MftError::InvalidDataRun("MFT byte capacity overflow"))?;
    if bytes / boot.file_record_size > 1u64 << 48 {
        return Err(MftError::InvalidDataRun(
            "MFT capacity exceeds the NTFS record number range",
        ));
    }
    Ok(ExtentMap::from_runs(
        runs,
        boot.cluster_size,
        boot.file_record_size,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raw_mft::layout::attribute::{
        NtfsAttributeHeader, NtfsAttributeType, NtfsNonResidentAttributeHeader,
    };
    use std::mem::size_of;
    use zerocopy::IntoBytes;

    fn synthetic_bootstrap(attributes: &[Vec<u8>]) -> Result<MftBootstrap, MftError> {
        let mut boot = vec![0; 512];
        boot[3..11].copy_from_slice(b"NTFS    ");
        boot[11..13].copy_from_slice(&512u16.to_le_bytes());
        boot[13] = 1;
        boot[48..56].copy_from_slice(&8u64.to_le_bytes());
        boot[64] = (-10i8) as u8;
        let geometry = BootSector::parse(&boot)?;
        let mut record = vec![0; 1024];
        record[..4].copy_from_slice(b"FILE");
        record[4..6].copy_from_slice(&42u16.to_le_bytes());
        record[6..8].copy_from_slice(&3u16.to_le_bytes());
        record[16..18].copy_from_slice(&1u16.to_le_bytes());
        record[20..22].copy_from_slice(&56u16.to_le_bytes());
        record[28..32].copy_from_slice(&1024u32.to_le_bytes());
        let mut end = 56;
        for attribute in attributes {
            record[end..end + attribute.len()].copy_from_slice(attribute);
            end += attribute.len();
        }
        record[end..end + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        record[24..28].copy_from_slice(&((end + 4) as u32).to_le_bytes());
        record[42..44].copy_from_slice(&0xabcd_u16.to_le_bytes());
        for (index, trailer) in [510, 1022].into_iter().enumerate() {
            let replacement = [record[trailer], record[trailer + 1]];
            record[44 + index * 2..46 + index * 2].copy_from_slice(&replacement);
            record[trailer..trailer + 2].copy_from_slice(&0xabcd_u16.to_le_bytes());
        }
        let source = Arc::new(crate::volume::sparse_fixture::SparseVolumeFixture::new(
            vec![(0, boot), (4096, record)],
        ));
        bootstrap_mft_state(&source.open()?, &geometry)
    }

    fn resident_list(value: &[u8]) -> Vec<u8> {
        let mut attribute = vec![0; 24 + value.len()];
        attribute[..4].copy_from_slice(&0x20u32.to_le_bytes());
        let size = attribute.len() as u32;
        attribute[4..8].copy_from_slice(&size.to_le_bytes());
        attribute[16..20].copy_from_slice(&(value.len() as u32).to_le_bytes());
        attribute[20..22].copy_from_slice(&24u16.to_le_bytes());
        attribute[24..].copy_from_slice(value);
        attribute
    }

    #[test]
    fn bootstrap_rejects_malformed_attribute_list_tail() {
        let data = build_nonresident_attr(64, &[0x11, 2, 8, 0]);
        let reference = crate::raw_mft::layout::attribute::AttributeListEntryHeader {
            type_id: 0x80,
            record_length: 26,
            attribute_name_length: 0,
            attribute_name_offset: 0,
            lowest_vcn: 0,
            file_reference: 1 << 48,
            attribute_id: 0,
        };
        let mut list = reference.as_bytes().to_vec();
        list.extend_from_slice(&[0; 26]);
        assert!(synthetic_bootstrap(&[data, resident_list(&list)]).is_err());
    }

    #[test]
    fn bootstrap_rejects_malformed_attribute_framing_after_data() {
        let data = build_nonresident_attr(64, &[0x11, 2, 8, 0]);
        assert!(synthetic_bootstrap(&[data, vec![0; 16]]).is_err());
    }

    #[test]
    fn bootstrap_rejects_malformed_bitmap_runlist() {
        let data = build_nonresident_attr(64, &[0x11, 2, 8, 0]);
        let mut bitmap = build_nonresident_attr(64, &[0x21, 1]);
        bitmap[..4].copy_from_slice(&0xb0u32.to_le_bytes());
        assert!(synthetic_bootstrap(&[data, bitmap]).is_err());
    }

    #[test]
    fn bootstrap_rejects_overflowing_mft_capacity() {
        let mut runs = vec![0x08];
        runs.extend_from_slice(&u64::MAX.to_le_bytes());
        runs.push(0);
        assert!(synthetic_bootstrap(&[build_nonresident_attr(64, &runs)]).is_err());
    }

    #[test]
    fn bootstrap_rejects_mft_capacity_outside_record_reference_space() {
        let mut runs = vec![0x08];
        runs.extend_from_slice(&((1u64 << 49) + 2).to_le_bytes());
        runs.push(0);
        assert!(synthetic_bootstrap(&[build_nonresident_attr(64, &runs)]).is_err());
    }

    #[test]
    fn bootstrap_rejects_missing_declared_data_extent() {
        let mut data = build_nonresident_attr(64, &[0x11, 2, 8, 0]);
        data[48..56].copy_from_slice(&2048u64.to_le_bytes());
        assert!(synthetic_bootstrap(&[data]).is_err());
    }

    #[test]
    fn bootstrap_accepts_split_data_and_absent_bitmap() {
        let data = build_nonresident_attr(64, &[0x11, 1, 8, 0]);
        let mut continuation = build_nonresident_attr(64, &[0x11, 1, 9, 0]);
        continuation[14..16].copy_from_slice(&1u16.to_le_bytes());
        continuation[16..24].copy_from_slice(&1i64.to_le_bytes());
        let bootstrap = synthetic_bootstrap(&[data, continuation]).expect("valid split data");
        assert_eq!(bootstrap.extent_map.record_count(), 1);
        assert!(bootstrap.bitmap.is_empty());
    }

    /// Build a non-resident `$DATA` attribute whose runlist starts at
    /// `data_runs_offset` and is followed by `runlist` bytes.
    fn build_nonresident_attr(data_runs_offset: u16, runlist: &[u8]) -> Vec<u8> {
        let header_size = size_of::<NtfsNonResidentAttributeHeader>();
        let total = header_size + runlist.len();
        let header = NtfsNonResidentAttributeHeader {
            attribute_header: NtfsAttributeHeader {
                type_id: NtfsAttributeType::Data as u32,
                length: total as u32,
                is_non_resident: 1,
                name_length: 0,
                name_offset: 0,
                flags: 0,
                id: 0,
            },
            lowest_vcn: 0,
            highest_vcn: 0,
            data_runs_offset,
            compression_unit_exponent: 0,
            _reserved: [0; 5],
            allocated_size: 0,
            data_size: 0,
            initialized_size: 0,
        };
        let mut buf = vec![0u8; total];
        buf[..header_size].copy_from_slice(header.as_bytes());
        buf[header_size..].copy_from_slice(runlist);
        buf
    }

    #[test]
    fn decodes_valid_nonresident_runlist() {
        let header_size = size_of::<NtfsNonResidentAttributeHeader>();
        // Single run: 0x21 length=5, offset=0x0234 -> LCN 564.
        let buf = build_nonresident_attr(header_size as u16, &[0x21, 0x05, 0x34, 0x02, 0x00]);
        let attr = NtfsAttribute::new(&buf).expect("attr");
        let runs = decode_nonresident_runs(&attr).expect("runs");
        assert_eq!(
            runs,
            vec![DataRun::Data {
                lcn: 564,
                clusters: 5
            }]
        );
    }

    #[test]
    fn rejects_runs_offset_past_attribute() {
        let header_size = size_of::<NtfsNonResidentAttributeHeader>();
        let runlist = [0x21u8, 0x05, 0x34, 0x02, 0x00];
        let total = header_size + runlist.len();
        // data_runs_offset points beyond the attribute's own bytes.
        let buf = build_nonresident_attr((total + 10) as u16, &runlist);
        let attr = NtfsAttribute::new(&buf).expect("attr");
        assert!(decode_nonresident_runs(&attr).is_err());
    }

    #[test]
    fn rejects_undecodable_runlist() {
        let header_size = size_of::<NtfsNonResidentAttributeHeader>();
        // Truncated run: header byte promises offset bytes that are absent.
        let buf = build_nonresident_attr(header_size as u16, &[0x21, 0x05]);
        let attr = NtfsAttribute::new(&buf).expect("attr");
        assert!(decode_nonresident_runs(&attr).is_err());
    }

    #[test]
    fn assembles_stream_extents_in_lowest_vcn_order() {
        let extents = vec![
            StreamExtent {
                lowest_vcn: 3,
                record_number: 11,
                attribute_id: 2,
                runs: vec![DataRun::Data {
                    lcn: 200,
                    clusters: 2,
                }],
            },
            StreamExtent {
                lowest_vcn: 0,
                record_number: 0,
                attribute_id: 1,
                runs: vec![DataRun::Data {
                    lcn: 100,
                    clusters: 3,
                }],
            },
        ];

        assert_eq!(
            assemble_stream_runs(&extents).expect("contiguous extents"),
            vec![
                DataRun::Data {
                    lcn: 100,
                    clusters: 3,
                },
                DataRun::Data {
                    lcn: 200,
                    clusters: 2,
                },
            ]
        );
    }

    #[test]
    fn rejects_stream_extent_vcn_gaps() {
        let extents = vec![
            StreamExtent {
                lowest_vcn: 0,
                record_number: 0,
                attribute_id: 1,
                runs: vec![DataRun::Data {
                    lcn: 100,
                    clusters: 2,
                }],
            },
            StreamExtent {
                lowest_vcn: 3,
                record_number: 11,
                attribute_id: 2,
                runs: vec![DataRun::Data {
                    lcn: 200,
                    clusters: 1,
                }],
            },
        ];

        assert!(assemble_stream_runs(&extents).is_err());
        assert_eq!(
            assemble_contiguous_prefix(&extents).expect("usable prefix"),
            vec![DataRun::Data {
                lcn: 100,
                clusters: 2,
            }]
        );
    }

    #[test]
    fn rejects_stream_extent_vcn_overlaps_even_for_prefix_assembly() {
        let extents = vec![
            StreamExtent {
                lowest_vcn: 0,
                record_number: 0,
                attribute_id: 1,
                runs: vec![DataRun::Data {
                    lcn: 100,
                    clusters: 3,
                }],
            },
            StreamExtent {
                lowest_vcn: 2,
                record_number: 11,
                attribute_id: 2,
                runs: vec![DataRun::Data {
                    lcn: 200,
                    clusters: 1,
                }],
            },
        ];

        assert!(assemble_stream_runs(&extents).is_err());
        assert!(assemble_contiguous_prefix(&extents).is_err());
    }

    #[test]
    fn target_loadedness_includes_source_record_number() {
        let streams = MftStreamRuns {
            base_file_reference: 7 << 48,
            data_extents: vec![StreamExtent {
                lowest_vcn: 4,
                record_number: 10,
                attribute_id: 2,
                runs: vec![DataRun::Data {
                    lcn: 100,
                    clusters: 1,
                }],
            }],
            data_size: 0,
            bitmap_extents: Vec::new(),
            bitmap_size: 0,
            attribute_list: None,
        };

        assert!(target_is_loaded(
            &streams,
            &AttributeListReference {
                type_id: NtfsAttributeType::Data as u32,
                lowest_vcn: 4,
                file_reference: 10,
                id: 2,
                name: Vec::new(),
            }
        ));
        assert!(!target_is_loaded(
            &streams,
            &AttributeListReference {
                type_id: NtfsAttributeType::Data as u32,
                lowest_vcn: 4,
                file_reference: 11,
                id: 2,
                name: Vec::new(),
            }
        ));
    }
}
