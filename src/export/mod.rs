//! Versioned export representations, independent of scanner implementation structs.
//!
//! Enable `serde` for DTOs, or `jsonl` for `JsonlWriter`. Decimal strings preserve
//! large integers in JSON consumers. NTFS names retain original UTF-16 on every
//! platform; supplied native paths retain their OS representation. See `docs/export.md`.

mod convert;
#[cfg(feature = "jsonl")]
mod writer;
#[cfg(feature = "jsonl")]
pub use writer::JsonlWriter;

use serde::{Deserialize, Serialize};

/// Presence and completeness of the supported metadata, not file contents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum FieldState {
    /// No reliable inspection evidence is available.
    Unknown,
    /// Collection was disabled by the caller.
    NotCollected,
    /// No value was observed; absence is not established.
    NotObserved,
    /// Absence was established by complete inspection.
    Absent,
    /// Supported metadata was inspected completely.
    Present,
    /// Some metadata was observed but inspection was incomplete.
    Partial,
}

/// Exact UTF-16 name or native platform representation of a supplied path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(tag = "encoding", content = "units", rename_all = "snake_case")]
pub enum NameUnits {
    /// NTFS or Windows UTF-16 units, including unpaired surrogates, on any host.
    WindowsUtf16(Vec<u16>),
    /// Unix OS bytes. These are not the original NTFS UTF-16 bytes.
    UnixBytes(Vec<u8>),
}

/// Lossless supplied name plus an explicitly non-authoritative readable form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct ExportName {
    /// Replacement characters may occur here; use `raw` for roundtrips.
    pub display: String,
    /// Exact supplied representation, independently readable on any platform.
    pub raw: NameUnits,
}

/// A full NTFS reference with its record and sequence fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct ExportReference {
    /// Full unsigned identifier as a decimal string.
    #[cfg_attr(test, schemars(regex(pattern = "^(0|[1-9][0-9]*)$")))]
    pub raw: String,
    /// Lower 48 bits of a standard reference, as a decimal string.
    #[cfg_attr(test, schemars(regex(pattern = "^(0|[1-9][0-9]*)$")))]
    pub record_number: String,
    /// Upper 16 bits of a standard reference.
    pub sequence_number: u16,
}

/// Why readable timestamp text is unavailable or ambiguous.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum TimestampStatus {
    /// Nonzero ticks formatted as UTC.
    Formatted,
    /// Zero may mean an unset/default value or the FILETIME epoch; consult metadata.
    ZeroAmbiguous,
    /// Outside four-digit-year RFC 3339 representation.
    OutOfRange,
}

/// A FILETIME with all 100-nanosecond ticks retained.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct ExportTimestamp {
    /// Unsigned ticks since 1601-01-01, as a decimal string.
    #[cfg_attr(test, schemars(regex(pattern = "^(0|[1-9][0-9]*)$")))]
    pub ticks: String,
    /// UTC RFC 3339 with exactly seven fractional digits, or null outside range.
    pub utc: Option<String>,
    /// Interpretation of the readable form.
    pub status: TimestampStatus,
}

/// Four timestamps from one attribute category.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct ExportTimes {
    /// Creation time.
    pub created: ExportTimestamp,
    /// File modification time.
    pub modified: ExportTimestamp,
    /// MFT record modification time.
    pub mft_modified: ExportTimestamp,
    /// Last access time.
    pub accessed: ExportTimestamp,
}

/// One retained file-name link. Namespace is the scanner's normalized numeric value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct ExportLink {
    /// Full parent reference, including sequence.
    pub parent: ExportReference,
    /// 0 POSIX, 1 Win32, 2 DOS, 3 Win32/DOS; original unknown codes are unavailable.
    pub namespace: u8,
    /// Leaf name.
    pub name: ExportName,
}

/// Supported stream size and residency metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct ExportStream {
    /// Null identifies the unnamed stream.
    pub name: Option<ExportName>,
    /// Logical size in bytes, as a decimal string.
    #[cfg_attr(test, schemars(regex(pattern = "^(0|[1-9][0-9]*)$")))]
    pub real_size: String,
    /// Allocated size in bytes, as reported by the scanner, as a decimal string.
    #[cfg_attr(test, schemars(regex(pattern = "^(0|[1-9][0-9]*)$")))]
    pub allocated_size: String,
    /// Whether data is resident in a FILE record.
    pub resident: bool,
}

/// Run summary of the represented unnamed stream segment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct ExportRuns {
    /// Number of runs.
    pub run_count: u32,
    /// Logical clusters, as a decimal string.
    #[cfg_attr(test, schemars(regex(pattern = "^(0|[1-9][0-9]*)$")))]
    pub total_clusters: String,
}

/// Category-level evidence; stored separately from default-valued scan fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct ExportMetadata {
    /// Standard-information attribute evidence.
    pub standard_information: FieldState,
    /// Names and selected-name timestamp evidence.
    pub file_names: FieldState,
    /// Unnamed data evidence.
    pub unnamed_data: FieldState,
    /// Named data evidence.
    pub alternate_data_streams: FieldState,
    /// Reparse-point attribute evidence, not a tag copied from FILE_NAME.
    pub reparse_point: FieldState,
    /// Run summary evidence.
    pub data_run_summary: FieldState,
    /// Whether DOS aliases were requested; null means unknown.
    pub dos_links_collected: Option<bool>,
}

/// Optional caller-supplied path evidence. Export never builds a directory tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ExportPath {
    /// No resolver was invoked.
    NotRequested,
    /// Resolver could not produce a path; a leaf name is not substituted.
    Unresolved,
    /// A complete caller-supplied path.
    Resolved {
        /// Supplied path, in OS encoding.
        path: ExportName,
    },
    /// A caller-supplied partial path.
    Partial {
        /// Supplied path fragment.
        path: ExportName,
    },
}

/// Stable error category plus best available source location.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct ExportError {
    /// `io`, `parse`, or `other`. Diagnostic text is not a machine contract.
    pub category: String,
    /// Human-readable original error.
    pub message: String,
    /// Record slot if known; never inferred from iterator position.
    #[cfg_attr(test, schemars(regex(pattern = "^(0|[1-9][0-9]*)$")))]
    pub record_number: Option<String>,
    /// Original volume byte offset if the error supplies one.
    #[cfg_attr(test, schemars(regex(pattern = "^(0|[1-9][0-9]*)$")))]
    pub volume_offset: Option<String>,
    /// Offset reported by an error without a known address space.
    #[cfg_attr(test, schemars(regex(pattern = "^(0|[1-9][0-9]*)$")))]
    pub reported_offset: Option<String>,
}

/// Structured metadata limitation with optional nested read error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct ExportDiagnostic {
    /// Physical record where the issue was observed.
    #[cfg_attr(test, schemars(regex(pattern = "^(0|[1-9][0-9]*)$")))]
    pub record_number: String,
    /// Affected NTFS type code, if known.
    pub attribute_type: Option<u32>,
    /// Stable snake_case issue code, documented in the schema guide.
    pub code: String,
    /// Full referenced file identifier for extension-read failures.
    pub reference: Option<ExportReference>,
    /// Original extension-read failure.
    pub error: Option<ExportError>,
}

/// Observed non-resident payload outside the logical MFT input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct ExportExternalPayload {
    /// Physical record number as decimal.
    #[cfg_attr(test, schemars(regex(pattern = "^(0|[1-9][0-9]*)$")))]
    pub record_number: String,
    /// NTFS attribute type code.
    pub attribute_type: u32,
    /// Instance ID within that record.
    pub attribute_id: u16,
}

/// Version-one record representation. This is not a serialized scanner struct.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct ExportRecordV1 {
    /// Adapter that supplied this record: `entry`, `mft_stream`, or `volume_scan`.
    /// `mft_stream` may read a live volume; it does not imply offline provenance.
    pub source: String,
    /// Supplied physical record number as decimal, independent of the reference.
    #[cfg_attr(test, schemars(regex(pattern = "^(0|[1-9][0-9]*)$")))]
    pub record_number: String,
    /// Supplied record-header sequence number.
    pub sequence_number: u16,
    /// Full identifier, including sequence.
    pub reference: ExportReference,
    /// Base record number as supplied by the scanner; zero is its base-record sentinel.
    #[cfg_attr(test, schemars(regex(pattern = "^(0|[1-9][0-9]*)$")))]
    pub base_record_number: String,
    /// Always null for current adapters: scanner has discarded base sequence bits.
    pub base_sequence_number: Option<u16>,
    /// Logical extracted-stream offset; never a volume offset.
    #[cfg_attr(test, schemars(regex(pattern = "^(0|[1-9][0-9]*)$")))]
    pub stream_offset: Option<String>,
    /// FILE header hard-link count; not necessarily equal to retained link count.
    pub hard_link_count: u16,
    /// Raw FILE header flags, including unknown bits.
    pub flags: u16,
    /// Record marked in use.
    pub used: bool,
    /// Record marked as a directory.
    pub directory: bool,
    /// Scanner's reparse-point indicator.
    pub reparse_point: bool,
    /// Scanner's tag, which may originate from FILE_NAME.
    pub reparse_tag: Option<u32>,
    /// Selected link; may contain defaults when names are absent or unknown.
    pub selected_name: ExportLink,
    /// Every link retained by the scanner, in original order.
    pub links: Vec<ExportLink>,
    /// Standard-information timestamps; interpret using metadata.
    pub standard_information_times: ExportTimes,
    /// Selected FILE_NAME timestamps only; other links' times are unavailable.
    pub file_name_times: ExportTimes,
    /// Raw SI attribute flags, including unknown bits.
    pub file_attributes: u32,
    /// Null when no unnamed data was observed, not necessarily proven absent.
    pub unnamed_stream: Option<ExportStream>,
    /// Scanner's unnamed-stream sparse indicator.
    pub sparse: bool,
    /// Scanner's unnamed-stream compressed indicator.
    pub compressed: bool,
    /// Scanner's unnamed-stream encrypted indicator.
    pub encrypted: bool,
    /// Summary when available; see metadata for completeness.
    pub data_runs: Option<ExportRuns>,
    /// Retained named streams, in original order; no deduplication or flattening.
    pub alternate_streams: Vec<ExportStream>,
    /// Evidence for presence, completeness, and collection choices.
    pub metadata: ExportMetadata,
    /// Selected-name path evidence supplied by the caller.
    pub path: ExportPath,
    /// Parsing diagnostics; empty entry diagnostics do not prove completeness.
    pub diagnostics: Vec<ExportDiagnostic>,
    /// Observed external payload headers. Empty does not establish absence.
    pub external_payloads: Vec<ExportExternalPayload>,
    /// `ntfs_utf16_preserved` for exact original NTFS code units.
    pub name_fidelity: String,
}

/// Versioned, independently decodable JSONL item. Unknown versions are rejected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(tag = "schema_version")]
pub enum ExportItem {
    /// The version-one schema.
    #[serde(rename = "1")]
    V1(ExportItemV1),
}

/// Kind of item in a version-one stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum ExportItemV1 {
    /// Successfully parsed metadata, potentially partial.
    Record(Box<ExportRecordV1>),
    /// Item-level scan error.
    Error(ExportError),
}

#[cfg(test)]
mod tests;
