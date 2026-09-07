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
