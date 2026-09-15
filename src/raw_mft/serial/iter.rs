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
/// errors are returned as items; a caller that continues may visit later slots.
/// Attribute-list enrichment errors are not returned. Successful collection
/// does not establish complete metadata. Use [`RawMft::scan_entries_diagnostic`]
/// for skip counts and evidence, or [`crate::MftStream`] for per-slot outcomes.
/// A live volume scan is not an atomic snapshot.
pub struct RawMftEntryScan<'a> {
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
            Err(error) => Some(Err(error)),
        }
    }
}

/// Opt-in diagnostic raw iterator. Corrupt records are counted and skipped;
/// scan I/O errors are returned once and stop iteration. Metadata limitations
/// accompany successful records. Use [`Self::report`] even after early stopping.
pub struct RawMftDiagnosticEntryScan<'a> {
    inner: RawMftEntryScan<'a>,
    stopped: bool,
}

impl RawMft<'_> {
    /// Scan base records with metadata evidence and bounded parser diagnostics.
    ///
    /// `diagnostic_limit` bounds retained skipped-record errors; all skips are
    /// still counted. This performs extra attribute validation per yielded record.
    pub fn scan_entries_diagnostic(
        &self,
        options: RawMftScanOptions,
        diagnostic_limit: usize,
    ) -> Result<RawMftDiagnosticEntryScan<'_>, MftError> {
        let mut inner = self.scan_entries_with_options(options)?;
        inner.scan.diagnostic_limit = Some(diagnostic_limit);
        Ok(RawMftDiagnosticEntryScan {
            inner,
            stopped: false,
        })
    }
}
impl RawMftDiagnosticEntryScan<'_> {
    /// Evidence accumulated so far. Early stopping leaves `reached_end` false.
    pub fn report(&self) -> &crate::RawMftScanReport {
        &self.inner.scan.report
    }
    /// Recover the report, including retained original parser errors.
    pub fn into_report(self) -> crate::RawMftScanReport {
        self.inner.scan.report
    }
}
impl Iterator for RawMftDiagnosticEntryScan<'_> {
    type Item = Result<crate::RawMftDiagnosticRecord, MftError>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.stopped {
            return None;
        }
        let inner = &mut self.inner;
        let result = next_record_output(inner.mft, &mut inner.scan, &mut inner.reader, |record| {
            use crate::raw_mft::inspection::{RecordInspection, inspect_view};
            use crate::{MftDiagnosticKind, MftFieldState};
            let mut inspection = RecordInspection::default();
            let attrs = inspect_view(record, &mut inspection);
            if attrs.iter().any(|a| a.type_id() == 0x20) {
                inspection.diagnose(
                    record.number,
                    None,
                    MftDiagnosticKind::EnrichmentNotVerified,
                );
            }
            let entry = build_rich_entry(
                inner.mft,
                record,
                &mut inner.attr_reader,
                inner.build_options,
            );
            let mut metadata = inspection.metadata();
            if !entry.collection_options.collect_alternate_data_streams() {
                metadata.alternate_data_streams = MftFieldState::NotCollected;
            }
            if !entry.collection_options.collect_dos_file_name_links()
                && matches!(metadata.file_names, MftFieldState::Present { .. })
            {
                metadata.file_names = MftFieldState::Present { complete: false };
            }
            Ok(crate::RawMftDiagnosticRecord {
                entry,
                metadata,
                diagnostics: inspection.into_diagnostics(),
            })
        });
        match result {
            Ok(Some(record)) => Some(Ok(record)),
            Ok(None) => {
                self.stopped = true;
                None
            }
            Err(error) => {
                self.stopped = true;
                Some(Err(error))
            }
        }
    }
}
impl std::iter::FusedIterator for RawMftDiagnosticEntryScan<'_> {}
