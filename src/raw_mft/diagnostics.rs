//! Opt-in raw scan diagnostics. Successful iteration is not a completeness guarantee.
use crate::{MftDiagnostic, MftError, MftMetadata, RawMftEntry};

/// One recoverable record-level failure observed during a raw scan.
#[derive(Debug)]
pub struct RawMftScanDiagnostic {
    /// Physical record slot at which parsing failed.
    pub record_number: u64,
    /// Original structured parser error.
    pub error: MftError,
}

/// Progress and skipped-record evidence for an opt-in diagnostic iterator.
///
/// Counts describe only slots actually visited, not unused bands excluded by
/// a separate chunk plan. Diagnostic storage is bounded by the requested limit.
#[derive(Debug, Default)]
pub struct RawMftScanReport {
    /// Slots visited, including skipped slots.
    pub visited: u64,
    /// Base records returned to the caller.
    pub yielded: u64,
    /// Slots excluded by the bitmap.
    pub skipped_unused: u64,
    /// Sparse logical slots without physical records.
    pub skipped_sparse: u64,
    /// Slots with no FILE signature or an implausible header.
    pub skipped_non_file: u64,
    /// Extension records omitted from the base-record view.
    pub skipped_extensions: u64,
    /// Plausible FILE records rejected by the parser.
    pub invalid_records: u64,
    /// Retained parser failures. Remaining failures are counted but not retained.
    pub diagnostics: Vec<RawMftScanDiagnostic>,
    /// Parser failures omitted after reaching the caller's diagnostic limit.
    pub omitted_diagnostics: u64,
    /// True only after the iterator reaches the end of its selected range.
    /// This does not imply all records or metadata were valid.
    pub reached_end: bool,
}

/// A raw base record with conservative, per-category metadata evidence.
///
/// Base attributes are inspected with the same checks as the offline parser.
/// An attribute list makes extension completeness unverified even when selected
/// fields were enriched. Original non-resident file payloads are not returned.
#[derive(Debug)]
pub struct RawMftDiagnosticRecord {
    /// Materialized entry and the collection choices that produced it.
    pub entry: RawMftEntry,
    /// Presence and completeness of inspected metadata categories.
    pub metadata: MftMetadata,
    /// Recoverable metadata limitations for this record.
    pub diagnostics: Vec<MftDiagnostic>,
}
