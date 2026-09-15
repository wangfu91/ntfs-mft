//! Read-only parsing of logical `$MFT` bytes from a file, buffer, or volume.

use super::inspection::{RecordInspection, inspect_view};
use super::{
    MftByteReader, RawMft, RawMftEntry,
    layout::{attribute::NtfsAttribute, record::FileRecord},
};
use crate::{
    MftDiagnostic, MftDiagnosticKind, MftError, MftExternalPayload, MftMetadata, MftResult,
};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{BufReader, Read, Seek, SeekFrom},
    path::Path,
};

/// Configuration for an extracted MFT stream. No geometry is guessed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MftStreamOptions {
    record_size: usize,
    resolve_extensions: bool,
}

impl MftStreamOptions {
    /// Configure a power-of-two record size between 512 bytes and 64 KiB.
    ///
    /// The bounded size prevents untrusted headers from causing large allocations.
    /// Extension resolution is disabled by default.
    pub fn new(record_size: usize) -> MftResult<Self> {
        if !(512..=65536).contains(&record_size) || !record_size.is_power_of_two() {
            return Err(MftError::InvalidOptions(
                "MFT stream record size must be a power of two between 512 and 65536",
            ));
        }
        Ok(Self {
            record_size,
            resolve_extensions: false,
        })
    }

    /// Resolve one level of resident `$ATTRIBUTE_LIST` references within the stream.
    #[must_use]
    pub const fn resolve_extensions(mut self, enabled: bool) -> Self {
        self.resolve_extensions = enabled;
        self
    }

    /// Configured record size in bytes.
    pub const fn record_size(self) -> usize {
        self.record_size
    }
}

/// Metadata and provenance for one physical MFT slot.
#[derive(Debug)]
pub struct MftStreamRecord {
    /// Existing metadata representation. Interpret default values using `metadata`.
    pub entry: RawMftEntry,
    /// Offset in the supplied stream, not an offset in the original volume.
    pub stream_offset: u64,
    /// Per-category presence and completeness.
    pub metadata: MftMetadata,
    /// Recoverable parsing and extension-resolution limitations.
    pub diagnostics: Vec<MftDiagnostic>,
    /// Observed non-resident attribute payloads unavailable in an MFT-only stream.
    /// Empty does not prove that none exist when metadata inspection was incomplete.
    pub external_payloads: Vec<MftExternalPayload>,
}

/// A generic, read-only parser for a logical `$MFT` stream.
/// File/buffer input is an offline MFT scan; volume input is a live volume scan.
///
/// Offset zero must be record zero; retain empty/unused slots. Input must contain
/// original USA-protected bytes, not already fixed-up records. The source must
/// remain unchanged while borrowed by this parser. Constructors seek to zero
/// regardless of the reader's initial position. `Cursor<&[u8]>`, `Cursor<Vec<u8>>`,
/// and files are supported. This API does not read arbitrary file contents.
///
/// ```no_run
/// use ntfs_mft::{MftResult, MftStream, MftStreamOptions};
/// use std::io::Cursor;
/// # fn main() -> MftResult<()> {
/// let options = MftStreamOptions::new(1024)?;
/// let mut file = MftStream::from_path("extracted.MFT", options)?;
/// let root = file.read_record(5)?;
/// let bytes = std::fs::read("extracted.MFT")?;
/// let mut memory = MftStream::from_reader(Cursor::new(bytes.as_slice()), options)?;
/// assert_eq!(root.entry, memory.read_record(5)?.entry);
/// # Ok(())
/// # }
/// ```
pub struct MftStream<R: Read + Seek> {
    reader: R,
    options: MftStreamOptions,
    record_count: u64,
}

impl MftStream<BufReader<File>> {
    /// Open an extracted MFT file read-only, with ordinary user file permissions.
    pub fn from_path(path: impl AsRef<Path>, options: MftStreamOptions) -> MftResult<Self> {
        Self::from_reader(BufReader::new(File::open(path)?), options)
    }
}

impl MftStream<MftByteReader> {
    /// Parse a volume's logical MFT using its discovered record size.
    ///
    /// The stream owns an independent handle and can outlive `raw` and its
    /// volume. Extension resolution is disabled by default, as with
    /// [`MftStreamOptions::new`]. For custom options, use [`Self::from_reader`]
    /// with [`super::RawMft::byte_reader`]. A live volume is not a snapshot.
    pub fn from_raw_mft(raw: &RawMft<'_>) -> MftResult<Self> {
        Self::from_reader(
            raw.byte_reader()?,
            MftStreamOptions::new(raw.file_record_size())?,
        )
    }
}

impl<R: Read + Seek> MftStream<R> {
    /// Create a parser, rejecting incomplete trailing records.
    /// Empty input is valid. FILE headers are validated when their slots are read.
    pub fn from_reader(mut reader: R, options: MftStreamOptions) -> MftResult<Self> {
        let size = reader.seek(SeekFrom::End(0))?;
        let record_size = options.record_size as u64;
        let remainder = size % record_size;
        if remainder != 0 {
            return Err(MftError::TruncatedRecord {
                offset: size - remainder,
                needed: options.record_size,
                got: remainder as usize,
            });
        }
        // NTFS file references carry a 48-bit slot number.
        if size / record_size > 1u64 << 48 {
            return Err(MftError::InvalidOptions(
                "MFT stream exceeds the NTFS record number range",
            ));
        }
        reader.seek(SeekFrom::Start(0))?;
        Ok(Self {
            reader,
            options,
            record_count: size / record_size,
        })
    }

    /// Number of complete physical slots, including non-FILE and unused slots.
    pub const fn record_count(&self) -> u64 {
        self.record_count
    }

    /// Configured FILE record size in bytes.
    pub const fn file_record_size(&self) -> usize {
        self.options.record_size
    }

    /// Recover the source; its cursor position is unspecified.
    pub fn into_inner(self) -> R {
        self.reader
    }

    fn load(&mut self, number: u64) -> MftResult<RecordData> {
        if number >= self.record_count {
            return Err(MftError::InvalidMftRecord {
                number,
                reason: "record number outside MFT stream",
            });
        }
        let offset =
            number
                .checked_mul(self.options.record_size as u64)
                .ok_or(MftError::InvalidOptions(
                    "MFT stream record offset overflow",
                ))?;
        self.reader.seek(SeekFrom::Start(offset))?;
        let mut bytes = vec![0; self.options.record_size];
        self.reader.read_exact(&mut bytes)?;
        let record = FileRecord::parse(number, None, &mut bytes)?;
        let header = record.header;
        let expected_usa = self.options.record_size / 512 + 1;
        if header.allocated_size as usize != self.options.record_size
            || header.update_sequence_length as usize != expected_usa
            || (header.attributes_offset as usize) < 42
            || (header.update_sequence_offset as usize) < 42
            || header.update_sequence_offset as usize + expected_usa * 2
                > header.attributes_offset as usize
        {
            return Err(MftError::InvalidMftRecord {
                number,
                reason: "MFT stream FILE geometry is inconsistent",
            });
        }
        Ok(RecordData {
            number,
            bytes,
            header,
        })
    }

    /// Read one slot. Non-FILE slots, bad USA fixups, and out-of-range numbers
    /// are errors; incomplete metadata retains the entry with diagnostics.
    pub fn read_record(&mut self, number: u64) -> MftResult<MftStreamRecord> {
        let base = self.load(number)?;
        let mut inspection = RecordInspection::default();
        let base_attrs = inspect_view(&base.view(), &mut inspection);
        let mut references = Vec::new();
        if base.header.base_reference != 0 {
            inspection.diagnose(number, None, MftDiagnosticKind::ExtensionRecordOnly);
        } else {
            for attr in base_attrs.iter().filter(|a| a.type_id() == 0x20) {
                if attr.is_non_resident() {
                    inspection.diagnose(
                        number,
                        None,
                        MftDiagnosticKind::ExternalPayloadUnavailable,
                    );
                } else if self.options.resolve_extensions {
                    references.extend(list_references(attr, number, &mut inspection));
                } else {
                    inspection.diagnose(number, None, MftDiagnosticKind::ExtensionsNotResolved);
                }
            }
        }
        let mut extensions = BTreeMap::new();
        for reference in &references {
            let ext_number = reference.file_reference & 0x0000_ffff_ffff_ffff;
            if ext_number == number || extensions.contains_key(&ext_number) {
                continue;
            }
            // Cache failures as well as successes to bound repeated lookups.
            let loaded = match self.load(ext_number) {
                Ok(record) => Some(record),
                Err(error) => {
                    inspection.diagnose(
                        ext_number,
                        None,
                        MftDiagnosticKind::ExtensionRead {
                            reference: reference.file_reference,
                            error,
                        },
                    );
                    None
                }
            };
            extensions.insert(ext_number, loaded);
        }
        let mut attributes = base_attrs;
        let mut selected = std::collections::BTreeSet::new();
        for reference in &references {
            let ext_number = reference.file_reference & 0x0000_ffff_ffff_ffff;
            let source = if ext_number == number {
                Some(&base)
            } else {
                extensions.get(&ext_number).and_then(Option::as_ref)
            };
            let Some(source) = source else {
                continue;
            };
            let expected_base = base.view().file_reference();
            if source.header.sequence_value != (reference.file_reference >> 48) as u16
                || (ext_number != number && source.header.base_reference != expected_base)
            {
                inspection.diagnose(
                    ext_number,
                    Some(reference.type_id),
                    MftDiagnosticKind::ExtensionMismatch,
                );
                continue;
            }
            // Scan only after reference validation; reject unrelated extension attributes.
            let mut local = RecordInspection::default();
            let candidates = inspect_view(&source.view(), &mut local);
            if ext_number != number && candidates.iter().any(|a| a.type_id() == 0x20) {
                inspection.diagnose(ext_number, None, MftDiagnosticKind::NestedAttributeList);
            }
            let matched = candidates.into_iter().find(|attr| reference.matches(attr));
            match matched {
                Some(attr) if ext_number != number => {
                    let key = (ext_number, attr.header.id);
                    if selected.insert(key) {
                        inspection.observe(ext_number, &attr);
                        attributes.push(attr);
                    }
                }
                Some(_) => {}
                None => inspection.diagnose(
                    ext_number,
                    Some(reference.type_id),
                    MftDiagnosticKind::ExtensionMismatch,
                ),
            }
            if local.has_malformed_attribute() {
                // A malformed extension may hide another required attribute. Do not
                // claim completeness even if the selected attribute was accessible.
                inspection.diagnose(
                    ext_number,
                    Some(reference.type_id),
                    MftDiagnosticKind::MalformedAttribute,
                );
            }
        }
        let entry = RawMftEntry::from_attributes(
            &base.view(),
            attributes.into_iter().filter(|attr| {
                attr.type_id() != 0x80
                    || attr.nonresident_header().is_none_or(|h| h.lowest_vcn == 0)
            }),
        );
        let metadata = inspection.metadata();
        let (diagnostics, external_payloads) = inspection.into_diagnostics_and_external_payloads();
        Ok(MftStreamRecord {
            entry,
            stream_offset: number * self.options.record_size as u64,
            metadata,
            diagnostics,
            external_payloads,
        })
    }

    /// Iterate every physical slot from zero. Parse errors consume one slot and
    /// iteration continues. An I/O error is returned once, then iteration stops.
    /// A recoverable extension I/O error is attached to its base entry and also
    /// stops this iterator after yielding that entry.
    pub fn iter_records(&mut self) -> MftStreamIter<'_, R> {
        MftStreamIter {
            stream: self,
            next: 0,
            stopped: false,
        }
    }
}

struct RecordData {
    number: u64,
    bytes: Vec<u8>,
    header: super::layout::record::FileRecordHeader,
}

impl RecordData {
    fn view(&self) -> FileRecord<'_> {
        FileRecord {
            data: &self.bytes,
            header: self.header,
            number: self.number,
        }
    }
}

struct ListReference {
    type_id: u32,
    file_reference: u64,
    id: u16,
    lowest_vcn: i64,
    name: Vec<u8>,
}

impl ListReference {
    fn matches(&self, attr: &NtfsAttribute<'_>) -> bool {
        self.type_id == attr.type_id()
            && self.id == attr.header.id
            && self.name.as_slice() == attr.name_bytes().unwrap_or_default()
            && self.lowest_vcn == attr.nonresident_header().map_or(0, |h| h.lowest_vcn)
    }
}

fn list_references(
    attr: &NtfsAttribute<'_>,
    number: u64,
    inspection: &mut RecordInspection,
) -> Vec<ListReference> {
    use super::layout::attribute::AttributeListEntryHeader;
    use zerocopy::FromBytes;

    let Some(data) = attr.resident_value() else {
        return Vec::new();
    };
    let mut result = Vec::new();
    let mut offset = 0;
    while offset < data.len() {
        let remaining = &data[offset..];
        // Some lists include zero-filled alignment padding after their entries.
        if remaining.len() < 8 && remaining.iter().all(|b| *b == 0) {
            break;
        }
        let header_size = std::mem::size_of::<AttributeListEntryHeader>();
        let header = remaining
            .get(..header_size)
            .and_then(|bytes| AttributeListEntryHeader::read_from_bytes(bytes).ok());
        let Some(h) = header else {
            inspection.diagnose(number, None, MftDiagnosticKind::MalformedAttributeList);
            break;
        };
        let len = h.record_length as usize;
        let name_start = h.attribute_name_offset as usize;
        let name_len = h.attribute_name_length as usize * 2;
        if len < header_size
            || len > remaining.len()
            || h.lowest_vcn < 0
            || (name_len != 0 && (name_start < header_size || name_start + name_len > len))
        {
            inspection.diagnose(number, None, MftDiagnosticKind::MalformedAttributeList);
            break;
        }
        result.push(ListReference {
            type_id: h.type_id,
            file_reference: h.file_reference,
            id: h.attribute_id,
            lowest_vcn: h.lowest_vcn,
            name: if name_len == 0 {
                Vec::new()
            } else {
                remaining[name_start..name_start + name_len].to_vec()
            },
        });
        offset += len;
    }
    result
}

/// Borrowing serial iterator over physical slots in a logical MFT stream.
pub struct MftStreamIter<'a, R: Read + Seek> {
    stream: &'a mut MftStream<R>,
    next: u64,
    stopped: bool,
}

impl<R: Read + Seek> Iterator for MftStreamIter<'_, R> {
    type Item = MftResult<MftStreamRecord>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.stopped || self.next >= self.stream.record_count {
            return None;
        }
        let result = self.stream.read_record(self.next);
        self.next += 1;
        self.stopped = match &result {
            Err(error) => error.is_io_error(),
            Ok(record) => record.diagnostics.iter().any(|d| {
                matches!(
                    &d.kind, MftDiagnosticKind::ExtensionRead { error, .. } if error.is_io_error()
                )
            }),
        };
        Some(result)
    }
}

impl<R: Read + Seek> std::iter::FusedIterator for MftStreamIter<'_, R> {}
