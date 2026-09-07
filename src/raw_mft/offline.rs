//! Offline, read-only parsing of a logical `$MFT` stream.

pub(in crate::raw_mft) mod inspect;
use super::{
    RawMftEntry,
    layout::{attribute::NtfsAttribute, record::FileRecord},
};
use crate::{MftError, MftResult};
use inspect::{Inspection, RecordData, inspect_record, list_references};
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
                "offline record size must be a power of two between 512 and 65536",
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

/// Whether metadata was observed and whether its supported attributes were inspected.
///
/// This describes metadata, never the availability of arbitrary file contents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OfflineFieldState {
    /// Collection was explicitly disabled for this metadata category.
    NotCollected,
    /// At least one valid attribute was observed. `complete` is false when more
    /// metadata may be missing or a split stream could not be fully summarized.
    Present {
        /// All relevant supported metadata was inspected successfully.
        complete: bool,
    },
    /// No such attribute exists in the fully inspected record set.
    Absent,
    /// No valid attribute was observed, but inspection was incomplete.
    Unknown,
}

/// Completeness of the metadata categories represented by [`RawMftEntry`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OfflineMetadata {
    /// `$STANDARD_INFORMATION` timestamps and flags.
    pub standard_information: OfflineFieldState,
    /// `$FILE_NAME` names, links, and timestamps.
    pub file_names: OfflineFieldState,
    /// Unnamed `$DATA` metadata, including its run summary.
    pub unnamed_data: OfflineFieldState,
    /// Named `$DATA` stream metadata.
    pub alternate_data_streams: OfflineFieldState,
    /// `$REPARSE_POINT` tag metadata (not a tag copied from `$FILE_NAME`).
    pub reparse_point: OfflineFieldState,
}

/// An attribute whose payload is outside the extracted MFT stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfflineExternalPayload {
    /// Physical MFT record containing the attribute header.
    pub record_number: u64,
    /// NTFS attribute type code.
    pub attribute_type: u32,
    /// Attribute instance identifier within that record.
    pub attribute_id: u16,
}

/// Why offline metadata is incomplete.
#[derive(Debug)]
#[non_exhaustive]
pub enum OfflineIssue {
    /// Raw scanning enriched selected fields but did not validate the complete
    /// extension attribute set. Presence may still be observed in the base.
    EnrichmentNotVerified,
    /// Attribute framing or a typed payload was malformed.
    MalformedAttribute,
    /// Extension lookup was disabled by the caller.
    ExtensionsNotResolved,
    /// A non-resident payload requires the original volume.
    ExternalPayloadUnavailable,
    /// A resident attribute list was malformed.
    MalformedAttributeList,
    /// A referenced record could not be read or validated.
    ExtensionRead {
        /// Full reference, including sequence number.
        reference: u64,
        /// Original item-level error.
        error: MftError,
    },
    /// The reference sequence, base reference, or selected attribute did not match.
    ExtensionMismatch,
    /// An extension has its own attribute list; recursive traversal is unsupported.
    NestedAttributeList,
    /// A data stream spans attribute segments; a complete run summary is unavailable.
    SplitDataStream,
    /// A physical extension record was requested without resolving its base.
    ExtensionRecordOnly,
}

/// A recoverable limitation associated with a parsed record.
#[derive(Debug)]
pub struct OfflineDiagnostic {
    /// Record at which the issue was observed.
    pub record_number: u64,
    /// Affected attribute type, or `None` when all categories may be affected.
    pub attribute_type: Option<u32>,
    /// Structured reason, including underlying errors where applicable.
    pub issue: OfflineIssue,
}

/// Metadata and provenance for one physical MFT slot.
#[derive(Debug)]
pub struct OfflineMftRecord {
    /// Existing metadata representation. Interpret default values using `metadata`.
    pub entry: RawMftEntry,
    /// Offset in the supplied stream, not an offset in the original volume.
    pub stream_offset: u64,
    /// Per-category presence and completeness.
    pub metadata: OfflineMetadata,
    /// Recoverable parsing and extension-resolution limitations.
    pub diagnostics: Vec<OfflineDiagnostic>,
    /// Observed non-resident attribute payloads unavailable in an MFT-only stream.
    /// Empty does not prove that none exist when metadata inspection was incomplete.
    pub external_payloads: Vec<OfflineExternalPayload>,
}

/// A generic, read-only parser for an extracted logical `$MFT` stream.
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
                "offline stream exceeds the NTFS record number range",
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
                reason: "record number outside offline stream",
            });
        }
        let offset = number
            .checked_mul(self.options.record_size as u64)
            .ok_or(MftError::InvalidOptions("offline record offset overflow"))?;
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
                reason: "offline FILE geometry is inconsistent",
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
    pub fn read_record(&mut self, number: u64) -> MftResult<OfflineMftRecord> {
        let base = self.load(number)?;
        let mut inspection = Inspection::default();
        let base_attrs = inspect_record(&base, &mut inspection);
        let mut references = Vec::new();
        if base.header.base_reference != 0 {
            inspection.issue(number, None, OfflineIssue::ExtensionRecordOnly);
        } else {
            for attr in base_attrs.iter().filter(|a| a.type_id() == 0x20) {
                if attr.is_non_resident() {
                    inspection.issue(number, None, OfflineIssue::ExternalPayloadUnavailable);
                } else if self.options.resolve_extensions {
                    references.extend(list_references(attr, number, &mut inspection));
                } else {
                    inspection.issue(number, None, OfflineIssue::ExtensionsNotResolved);
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
                    inspection.issue(
                        ext_number,
                        None,
                        OfflineIssue::ExtensionRead {
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
                inspection.issue(
                    ext_number,
                    Some(reference.type_id),
                    OfflineIssue::ExtensionMismatch,
                );
                continue;
            }
            // Scan only after reference validation; reject unrelated extension attributes.
            let mut local = Inspection::default();
            let candidates = inspect_record(source, &mut local);
            if ext_number != number && candidates.iter().any(|a| a.type_id() == 0x20) {
                inspection.issue(ext_number, None, OfflineIssue::NestedAttributeList);
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
                None => inspection.issue(
                    ext_number,
                    Some(reference.type_id),
                    OfflineIssue::ExtensionMismatch,
                ),
            }
            if local
                .diagnostics
                .iter()
                .any(|diagnostic| matches!(diagnostic.issue, OfflineIssue::MalformedAttribute))
            {
                // A malformed extension may hide another required attribute. Do not
                // claim completeness even if the selected attribute was accessible.
                inspection.issue(
                    ext_number,
                    Some(reference.type_id),
                    OfflineIssue::MalformedAttribute,
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
        Ok(OfflineMftRecord {
            entry,
            stream_offset: number * self.options.record_size as u64,
            metadata: inspection.metadata(),
            diagnostics: inspection.diagnostics,
            external_payloads: inspection.external_payloads,
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

/// Borrowing serial iterator over offline MFT slots.
pub struct MftStreamIter<'a, R: Read + Seek> {
    stream: &'a mut MftStream<R>,
    next: u64,
    stopped: bool,
}

impl<R: Read + Seek> Iterator for MftStreamIter<'_, R> {
    type Item = MftResult<OfflineMftRecord>;
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
                    &d.issue, OfflineIssue::ExtensionRead { error, .. } if error.is_io_error()
                )
            }),
        };
        Some(result)
    }
}

impl<R: Read + Seek> std::iter::FusedIterator for MftStreamIter<'_, R> {}
