//! Serial raw-MFT iteration over logical record numbers.

use crate::{
    errors::MftError,
    raw_mft::{
        RawMft,
        entry_build::{EntryBuildOptions, RawMftEntry},
        options::RawMftScanOptions,
        reader::{build_rich_entry, entry_build_options},
        volume_reader::VolumeReader,
    },
};

use super::engine::{SerialParseState, next_record_output};

/// Streaming best-effort iterator over base records in a live volume scan.
///
/// Default selection starts at record 24 and excludes bitmap-unused slots.
/// Sparse, non-FILE, malformed, and extension records are skipped. Scan I/O
/// errors are returned once, then iteration stops.
/// Attribute-list enrichment errors are not returned. Successful collection
/// does not establish complete metadata. Use [`crate::MftStream`] for per-slot
/// outcomes and metadata evidence.
/// A live volume scan is not an atomic snapshot.
pub struct RawMftEntryScan<'a> {
    stopped: bool,
    /// Parent raw-MFT reader.
    mft: &'a RawMft<'a>,
    /// Sector-aligned volume reader reused across iteration.
    reader: VolumeReader,
    /// Separate reader for random extension-record lookups so attr-list
    /// fixups do not mutate the iterator's sequential buffer window.
    attr_reader: VolumeReader,
    /// Shared serial scan state for the record walk.
    scan: SerialParseState,
    /// Cached entry-build options derived from the active iterator options.
    build_options: EntryBuildOptions,
}

impl<'a> RawMft<'a> {
    /// Create a lazy live-volume entry scan with default selection and the
    /// error policy of [`RawMftEntryScan`].
    #[must_use = "entry scans are lazy and do nothing unless consumed"]
    pub fn scan_entries(&self) -> Result<RawMftEntryScan<'_>, MftError> {
        self.scan_entries_with_options(RawMftScanOptions::default())
    }

    /// Create a lazy live-volume entry scan with custom selection and the
    /// error policy of [`RawMftEntryScan`].
    #[must_use = "entry scans are lazy and do nothing unless consumed"]
    pub fn scan_entries_with_options(
        &self,
        options: RawMftScanOptions,
    ) -> Result<RawMftEntryScan<'_>, MftError> {
        let (reader, attr_reader) = self.buffered_readers_for_options(&options)?;
        let scan = SerialParseState::from_options(self, &options);
        let build_options = entry_build_options(&options);

        Ok(RawMftEntryScan {
            stopped: false,
            mft: self,
            reader,
            attr_reader,
            scan,
            build_options,
        })
    }
}

impl<'a> Iterator for RawMftEntryScan<'a> {
    type Item = Result<RawMftEntry, MftError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.stopped {
            return None;
        }
        match next_record_output(self.mft, &mut self.scan, &mut self.reader, |record| {
            Ok(build_rich_entry(
                self.mft,
                record,
                &mut self.attr_reader,
                self.build_options,
            ))
        }) {
            Ok(Some(entry)) => Some(Ok(entry)),
            Ok(None) => None,
            Err(error) => {
                self.stopped = true;
                Some(Err(error))
            }
        }
    }
}

impl std::iter::FusedIterator for RawMftEntryScan<'_> {}

impl std::fmt::Debug for RawMftEntryScan<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RawMftEntryScan")
            .field("mft", &self.mft)
            .field("stopped", &self.stopped)
            .finish_non_exhaustive()
    }
}
