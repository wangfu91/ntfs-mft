//! Serial raw-MFT iteration over logical record numbers.

use crate::{
    errors::MftError,
    raw_mft::{
        RawMft,
        entry_build::{EntryBuildOptions, RawMftEntry},
        io::VolumeReader,
        options::RawMftScanOptions,
        reader::{build_rich_entry, entry_build_options},
    },
};

use super::engine::{SerialParseState, next_record_output};

/// Streaming iterator over MFT records.
pub struct RawMftIter<'a> {
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
    /// Begin iteration with default options.
    #[must_use = "iterators are lazy and do nothing unless consumed"]
    pub fn try_iter(&self) -> Result<RawMftIter<'_>, MftError> {
        self.try_iter_with_options(RawMftScanOptions::default())
    }

    /// Begin iteration with custom options.
    #[must_use = "iterators are lazy and do nothing unless consumed"]
    pub fn try_iter_with_options(
        &self,
        options: RawMftScanOptions,
    ) -> Result<RawMftIter<'_>, MftError> {
        let (reader, attr_reader) = self.buffered_readers_for_options(&options)?;
        let scan = SerialParseState::from_options(self, &options);
        let build_options = entry_build_options(&options);

        Ok(RawMftIter {
            mft: self,
            reader,
            attr_reader,
            scan,
            build_options,
        })
    }
}

impl<'a> Iterator for RawMftIter<'a> {
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
pub struct RawMftDiagnosticIter<'a> {
    inner: RawMftIter<'a>,
    stopped: bool,
}

impl RawMft<'_> {
    /// Scan base records with metadata evidence and bounded parser diagnostics.
    ///
    /// `diagnostic_limit` bounds retained skipped-record errors; all skips are
    /// still counted. This performs extra attribute validation per yielded record.
    pub fn try_iter_diagnostic(
        &self,
        options: RawMftScanOptions,
        diagnostic_limit: usize,
    ) -> Result<RawMftDiagnosticIter<'_>, MftError> {
        let mut inner = self.try_iter_with_options(options)?;
        inner.scan.diagnostic_limit = Some(diagnostic_limit);
        Ok(RawMftDiagnosticIter {
            inner,
            stopped: false,
        })
    }
}
impl RawMftDiagnosticIter<'_> {
    /// Evidence accumulated so far. Early stopping leaves `reached_end` false.
    pub fn report(&self) -> &crate::RawMftScanReport {
        &self.inner.scan.report
    }
    /// Recover the report, including retained original parser errors.
    pub fn into_report(self) -> crate::RawMftScanReport {
        self.inner.scan.report
    }
}
impl Iterator for RawMftDiagnosticIter<'_> {
    type Item = Result<crate::RawMftDiagnosticRecord, MftError>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.stopped {
            return None;
        }
        let inner = &mut self.inner;
        let result = next_record_output(inner.mft, &mut inner.scan, &mut inner.reader, |record| {
            use crate::raw_mft::offline::inspect::{Inspection, inspect_view};
            use crate::{OfflineFieldState, OfflineIssue};
            let mut inspection = Inspection::default();
            let attrs = inspect_view(record, &mut inspection);
            if attrs.iter().any(|a| a.type_id() == 0x20) {
                inspection.issue(record.number, None, OfflineIssue::EnrichmentNotVerified);
            }
            let entry = build_rich_entry(
                inner.mft,
                record,
                &mut inner.attr_reader,
                inner.build_options,
            );
            let mut metadata = inspection.metadata();
            if !entry.collection_options.collect_alternate_data_streams() {
                metadata.alternate_data_streams = OfflineFieldState::NotCollected;
            }
            // A requested run summary may be unavailable; avoid claiming complete
            // unnamed metadata when its summary was deliberately not inspected.
            if !entry.collection_options.collect_data_run_summary()
                && matches!(metadata.unnamed_data, OfflineFieldState::Present { .. })
                && !entry.is_resident
            {
                metadata.unnamed_data = OfflineFieldState::Present { complete: false };
            }
            if !entry.collection_options.collect_dos_file_name_links()
                && matches!(metadata.file_names, OfflineFieldState::Present { .. })
            {
                metadata.file_names = OfflineFieldState::Present { complete: false };
            }
            Ok(crate::RawMftDiagnosticRecord {
                entry,
                metadata,
                diagnostics: inspection.diagnostics,
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
impl std::iter::FusedIterator for RawMftDiagnosticIter<'_> {}
