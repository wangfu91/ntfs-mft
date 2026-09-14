//! Builder-configurable options for raw `$MFT` scans.

use std::num::NonZeroUsize;

use crate::raw_mft::{
    DEFAULT_ATTR_BUFFER_BYTES, DEFAULT_BUFFER_BYTES, layout::record::FIRST_NORMAL_RECORD,
};

/// Inclusive/exclusive logical record range for a raw `$MFT` scan.
///
/// The default is `0..`; the scan options separately default to `24..`.
/// Standard Rust ranges, including inclusive ranges, convert into this type.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RawMftRecordRange {
    /// First record number to include.
    pub(crate) start_record: u64,
    /// Exclusive end record number; `None` means up to `RawMft::record_count()`.
    pub(crate) end_record: Option<u64>,
}

impl RawMftRecordRange {
    /// Build a range from an inclusive start and optional exclusive end.
    #[must_use]
    pub const fn new(start_record: u64, end_record: Option<u64>) -> Self {
        Self {
            start_record,
            end_record,
        }
    }

    /// Inclusive first record number.
    #[must_use]
    pub const fn start_record(&self) -> u64 {
        self.start_record
    }

    /// Exclusive end record number, or `None` for the full `$MFT`.
    #[must_use]
    pub const fn end_record(&self) -> Option<u64> {
        self.end_record
    }
}

/// Buffer sizes used by raw `$MFT` scans.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawMftReadBuffers {
    /// Size of the I/O buffer in bytes used for batched reads of FILE records.
    pub(crate) main: NonZeroUsize,
    /// Size of the I/O buffer in bytes used for random attribute-list extension reads.
    pub(crate) attr: NonZeroUsize,
}

impl Default for RawMftReadBuffers {
    fn default() -> Self {
        Self {
            main: DEFAULT_BUFFER_BYTES,
            attr: DEFAULT_ATTR_BUFFER_BYTES,
        }
    }
}

impl RawMftReadBuffers {
    /// Build explicit main and attribute-list read buffers.
    #[must_use]
    pub const fn new(main: NonZeroUsize, attr: NonZeroUsize) -> Self {
        Self { main, attr }
    }

    /// Main sequential FILE-record scan buffer size in bytes.
    #[must_use]
    pub const fn main(&self) -> NonZeroUsize {
        self.main
    }

    /// Random extension-record read buffer size in bytes.
    #[must_use]
    pub const fn attr(&self) -> NonZeroUsize {
        self.attr
    }
}

/// Metadata materialization choices for raw `$MFT` entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RawMftEntryOptions {
    /// When false, skip collecting named alternate data streams.
    pub(crate) collect_alternate_data_streams: bool,
    /// When false, omit DOS 8.3 `$FILE_NAME` links shadowed by a better same-parent name.
    pub(crate) collect_dos_file_name_links: bool,
}

impl Default for RawMftEntryOptions {
    fn default() -> Self {
        Self {
            collect_alternate_data_streams: true,
            collect_dos_file_name_links: true,
        }
    }
}

impl RawMftEntryOptions {
    /// Configure reusable metadata materialization choices independently of a scan.
    pub fn builder() -> RawMftEntryOptionsBuilder {
        RawMftEntryOptionsBuilder::default()
    }

    /// Whether named alternate data streams are collected.
    #[must_use]
    pub const fn collect_alternate_data_streams(&self) -> bool {
        self.collect_alternate_data_streams
    }

    /// Whether shadowed DOS 8.3 `$FILE_NAME` links are retained.
    #[must_use]
    pub const fn collect_dos_file_name_links(&self) -> bool {
        self.collect_dos_file_name_links
    }
}

/// Builder for reusable rich-entry metadata choices.
#[derive(Debug, Clone, Default)]
#[must_use]
pub struct RawMftEntryOptionsBuilder {
    inner: RawMftEntryOptions,
}

impl RawMftEntryOptionsBuilder {
    /// Whether named alternate data streams are collected.
    pub fn collect_alternate_data_streams(mut self, value: bool) -> Self {
        self.inner.collect_alternate_data_streams = value;
        self
    }

    /// Whether shadowed DOS 8.3 links are retained.
    pub fn collect_dos_file_name_links(mut self, value: bool) -> Self {
        self.inner.collect_dos_file_name_links = value;
        self
    }

    /// Finalize the reusable policy.
    #[must_use]
    pub fn build(self) -> RawMftEntryOptions {
        self.inner
    }
}

/// Options controlling raw `$MFT` scan behavior.
///
/// Use [`RawMftScanOptions::builder`] for the fluent builder API.
/// Serial iteration and [`super::RawMftParallelScan`] use the same range and
/// unused-record selection. Parallel scans also apply selection during planning.
#[derive(Debug, Clone)]
pub struct RawMftScanOptions {
    /// Read-buffer sizing.
    pub(crate) buffers: RawMftReadBuffers,
    /// Logical record range.
    pub(crate) range: RawMftRecordRange,
    /// Entry materialization choices.
    pub(crate) entry: RawMftEntryOptions,
    /// Whether raw `$MFT` scans should include records marked unused in the `$MFT` `$BITMAP`.
    pub(crate) include_unused_records: bool,
}

impl Default for RawMftScanOptions {
    fn default() -> Self {
        Self {
            buffers: RawMftReadBuffers::default(),
            range: RawMftRecordRange::new(FIRST_NORMAL_RECORD, None),
            entry: RawMftEntryOptions::default(),
            include_unused_records: false,
        }
    }
}

impl RawMftScanOptions {
    /// Returns a fluent builder for [`RawMftScanOptions`].
    pub fn builder() -> RawMftScanOptionsBuilder {
        RawMftScanOptionsBuilder::default()
    }

    /// Read-buffer sizing.
    #[must_use]
    pub const fn buffers(&self) -> RawMftReadBuffers {
        self.buffers
    }

    /// Logical record range.
    #[must_use]
    pub const fn range(&self) -> RawMftRecordRange {
        self.range
    }

    /// Entry materialization choices.
    #[must_use]
    pub const fn entry(&self) -> RawMftEntryOptions {
        self.entry
    }

    /// Whether records marked unused in the `$MFT` bitmap are still returned.
    #[must_use]
    pub const fn include_unused_records(&self) -> bool {
        self.include_unused_records
    }
}

/// Fluent builder for [`RawMftScanOptions`].
#[derive(Debug, Default, Clone)]
#[must_use]
pub struct RawMftScanOptionsBuilder {
    /// Mutable options value being configured by the builder.
    inner: RawMftScanOptions,
}

impl RawMftScanOptionsBuilder {
    /// Set the I/O buffer size in bytes.
    pub fn buffer_bytes(mut self, v: NonZeroUsize) -> Self {
        self.inner.buffers.main = v;
        self
    }

    /// Set the random attribute-list extension-read buffer size in bytes.
    pub fn attr_buffer_bytes(mut self, v: NonZeroUsize) -> Self {
        self.inner.buffers.attr = v;
        self
    }

    /// Set both read-buffer sizes at once.
    pub fn buffers(mut self, v: RawMftReadBuffers) -> Self {
        self.inner.buffers = v;
        self
    }

    /// Set the logical scan range.
    pub fn range(mut self, v: impl Into<RawMftRecordRange>) -> Self {
        self.inner.range = v.into();
        self
    }

    /// Whether raw `$MFT` scans should include records marked unused in the `$MFT` bitmap.
    pub fn include_unused_records(mut self, v: bool) -> Self {
        self.inner.include_unused_records = v;
        self
    }

    /// Whether named alternate data streams should be collected.
    pub fn collect_alternate_data_streams(mut self, v: bool) -> Self {
        self.inner.entry.collect_alternate_data_streams = v;
        self
    }

    /// Whether DOS 8.3 `$FILE_NAME` links shadowed by better same-parent names should be collected.
    pub fn collect_dos_file_name_links(mut self, v: bool) -> Self {
        self.inner.entry.collect_dos_file_name_links = v;
        self
    }

    /// Set all entry materialization choices at once.
    pub fn entry(mut self, v: RawMftEntryOptions) -> Self {
        self.inner.entry = v;
        self
    }

    /// Set the inclusive starting record number.
    pub fn start_record(mut self, v: u64) -> Self {
        self.inner.range.start_record = v;
        self
    }

    /// Set the exclusive end record number, or `None` to iterate the full MFT.
    pub fn end_record(mut self, v: Option<u64>) -> Self {
        self.inner.range.end_record = v;
        self
    }

    /// Finalize the builder.
    #[must_use]
    pub fn build(self) -> RawMftScanOptions {
        self.inner
    }
}

/// Options for lean entry scanning. ADS settings are deliberately unavailable
/// because lean entries do not carry those fields.
#[derive(Debug, Clone, Default)]
pub struct RawMftLeanScanOptions {
    pub(crate) inner: RawMftScanOptions,
}

impl RawMftLeanScanOptions {
    /// Configure lean scan selection, buffers, and name-link retention.
    pub fn builder() -> RawMftLeanScanOptionsBuilder {
        RawMftLeanScanOptionsBuilder::default()
    }
    /// Read-buffer sizing.
    pub fn buffers(&self) -> RawMftReadBuffers {
        self.inner.buffers()
    }
    /// Logical record range.
    pub fn range(&self) -> RawMftRecordRange {
        self.inner.range()
    }
    /// Whether bitmap-unused records are included.
    pub fn include_unused_records(&self) -> bool {
        self.inner.include_unused_records()
    }
    /// Whether shadowed DOS links are retained.
    pub fn collect_dos_file_name_links(&self) -> bool {
        self.inner.entry.collect_dos_file_name_links
    }
}

impl RawMftScanOptions {
    /// Keep selection, buffers and DOS-link policy for a lean scan, explicitly
    /// discarding ADS materialization choices.
    pub fn into_lean(self) -> RawMftLeanScanOptions {
        RawMftLeanScanOptions { inner: self }
    }
}

/// Fluent builder accepting only options applicable to lean entries.
#[derive(Debug, Clone, Default)]
#[must_use]
pub struct RawMftLeanScanOptionsBuilder {
    inner: RawMftScanOptionsBuilder,
}

impl RawMftLeanScanOptionsBuilder {
    /// Set the main read buffer size.
    pub fn buffer_bytes(mut self, value: NonZeroUsize) -> Self {
        self.inner = self.inner.buffer_bytes(value);
        self
    }
    /// Set the extension-read buffer size.
    pub fn attr_buffer_bytes(mut self, value: NonZeroUsize) -> Self {
        self.inner = self.inner.attr_buffer_bytes(value);
        self
    }
    /// Set both read buffers.
    pub fn buffers(mut self, value: RawMftReadBuffers) -> Self {
        self.inner = self.inner.buffers(value);
        self
    }
    /// Set the logical record range.
    pub fn range(mut self, value: impl Into<RawMftRecordRange>) -> Self {
        self.inner = self.inner.range(value);
        self
    }
    /// Set the inclusive starting record.
    pub fn start_record(mut self, value: u64) -> Self {
        self.inner = self.inner.start_record(value);
        self
    }
    /// Set the exclusive ending record.
    pub fn end_record(mut self, value: Option<u64>) -> Self {
        self.inner = self.inner.end_record(value);
        self
    }
    /// Include records marked unused in the bitmap.
    pub fn include_unused_records(mut self, value: bool) -> Self {
        self.inner = self.inner.include_unused_records(value);
        self
    }
    /// Retain shadowed DOS name links.
    pub fn collect_dos_file_name_links(mut self, value: bool) -> Self {
        self.inner = self.inner.collect_dos_file_name_links(value);
        self
    }
    /// Finalize lean scan options.
    pub fn build(self) -> RawMftLeanScanOptions {
        self.inner.build().into_lean()
    }
}

impl From<std::ops::Range<u64>> for RawMftRecordRange {
    fn from(range: std::ops::Range<u64>) -> Self {
        Self::new(range.start, Some(range.end))
    }
}
impl From<std::ops::RangeFrom<u64>> for RawMftRecordRange {
    fn from(range: std::ops::RangeFrom<u64>) -> Self {
        Self::new(range.start, None)
    }
}
impl From<std::ops::RangeTo<u64>> for RawMftRecordRange {
    fn from(range: std::ops::RangeTo<u64>) -> Self {
        Self::new(0, Some(range.end))
    }
}
impl From<std::ops::RangeFull> for RawMftRecordRange {
    fn from(_: std::ops::RangeFull) -> Self {
        Self::default()
    }
}
impl From<std::ops::RangeInclusive<u64>> for RawMftRecordRange {
    fn from(range: std::ops::RangeInclusive<u64>) -> Self {
        if range.is_empty() {
            Self::new(*range.start(), Some(*range.start()))
        } else {
            Self::new(*range.start(), range.end().checked_add(1))
        }
    }
}
impl From<std::ops::RangeToInclusive<u64>> for RawMftRecordRange {
    fn from(range: std::ops::RangeToInclusive<u64>) -> Self {
        Self::new(0, range.end.checked_add(1))
    }
}

#[cfg(test)]
mod range_tests {
    use super::*;
    #[test]
    fn standard_ranges_keep_boundaries_and_scan_defaults() {
        for (range, start, end) in [
            ((100..200).into(), 100, Some(200)),
            ((24..).into(), 24, None),
            ((..).into(), 0, None),
            ((..10).into(), 0, Some(10)),
            ((2..=4).into(), 2, Some(5)),
            ((..=4).into(), 0, Some(5)),
            ((..=u64::MAX).into(), 0, None),
        ] {
            let range: RawMftRecordRange = range;
            assert_eq!((range.start_record(), range.end_record()), (start, end));
        }
        let mut exhausted = 3..=3;
        assert_eq!(exhausted.next(), Some(3));
        let range = RawMftRecordRange::from(exhausted);
        assert_eq!(range.start_record(), range.end_record().unwrap());
        assert_eq!(RawMftRecordRange::default().start_record(), 0);
        assert_eq!(RawMftScanOptions::default().range().start_record(), 24);
        assert_eq!(RawMftLeanScanOptions::default().range().start_record(), 24);
        assert_eq!(
            RawMftScanOptions::builder()
                .range(100..200)
                .build()
                .range()
                .end_record(),
            Some(200)
        );
        assert_eq!(
            RawMftLeanScanOptions::builder()
                .range(..)
                .build()
                .range()
                .start_record(),
            0
        );
    }
}

#[cfg(test)]
mod entry_options_tests {
    use super::*;

    #[test]
    fn standalone_entry_policy_preserves_defaults_and_reuses_in_scan() {
        assert_eq!(
            RawMftEntryOptions::builder().build(),
            RawMftEntryOptions::default()
        );
        let entry = RawMftEntryOptions::builder()
            .collect_alternate_data_streams(false)
            .collect_dos_file_name_links(false)
            .build();
        assert!(!entry.collect_alternate_data_streams());
        assert!(!entry.collect_dos_file_name_links());
        assert_eq!(
            RawMftScanOptions::builder().entry(entry).build().entry(),
            entry
        );
    }
}
