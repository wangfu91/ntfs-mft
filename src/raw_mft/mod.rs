//! Raw `$MFT` reader for NTFS volumes.
//!
//! This module reads the `$MFT` file directly from the volume and parses
//! each FILE record to expose rich per-record metadata (full timestamps, real
//! and allocated size, hard link count, alternate data streams, sparse
//! / compressed flags, file-name namespace, etc.).
//!
//! Quick start:
//!
//! ```no_run
//! use ntfs_mft::{MftResult, Volume};
//!
//! # fn main() -> MftResult<()> {
//! # #[cfg(windows)] {
//! let volume = Volume::from_drive_letter('C')?;
//! let mft = volume.raw_mft()?;
//! for entry in mft.scan_entries()? {
//!     let entry = entry?;
//!     if entry.is_used {
//!         println!("{:>8}: {}", entry.record_number, entry.file_name);
//!     }
//! }
//! # }
//! # Ok(())
//! # }
//! ```
//!
//! ## Limitations
//!
//! * NTFS only — ReFS volumes return [`crate::errors::MftError::UnsupportedFilesystem`].
//! * `$ATTRIBUTE_LIST` enrichment is intentionally one level deep; extension
//!   records are loaded when needed but are not recursively enriched.
//! * Windows requires Administrator privileges; Linux and macOS require read permission
//!   for the backing block device. All opens are read-only.

mod attr_list;
mod bootstrap;
mod byte_reader;
mod chunk_plan;
mod metadata;
pub use metadata::{
    MftDiagnostic, MftDiagnosticKind, MftExternalPayload, MftFieldState, MftMetadata,
};
// Enabled only by the non-publishable differential test package's build script.
#[cfg(all(test, ntfs_mft_differential))]
#[path = "../../differential-tests/tests/mod.rs"]
mod differential_tests;
mod entry_build;
pub(in crate::raw_mft) mod inspection;
mod layout;
mod options;
mod parallel;
mod reader;
mod serial;
mod stream;
#[cfg(test)]
mod tests;
mod volume_reader;

use std::num::NonZeroUsize;
use std::sync::Arc;

use crate::{
    errors::MftError,
    raw_mft::{
        entry_build::EntryBuildOptions,
        layout::{boot::BootSector, extent::ExtentMap},
        reader::read_record_at,
        volume_reader::VolumeReader,
    },
    volume::Volume,
};

pub use byte_reader::MftByteReader;
pub use chunk_plan::{RawMftChunkPlanOptions, RawMftChunkPlanOptionsBuilder, RawMftWorkChunk};
pub use entry_build::{AdsInfo, RawMftChunkBatch, RawMftEntry, RawMftLeanEntry, RawMftLink};
pub use layout::attribute::FileNameNamespace;
pub use options::{
    RawMftEntryOptions, RawMftEntryOptionsBuilder, RawMftLeanScanOptions,
    RawMftLeanScanOptionsBuilder, RawMftReadBuffers, RawMftRecordRange, RawMftScanOptions,
    RawMftScanOptionsBuilder,
};
pub use parallel::RawMftParallelScheduling;
pub use parallel::{Lean, RawMftParallelScan, Rich, Unselected};
pub use serial::RawMftEntryScan;
pub use stream::{MftRecord, MftRecordIter, MftStream, MftStreamOptions};

/// Default I/O buffer size for raw `$MFT` iteration.
#[allow(clippy::useless_nonzero_new_unchecked)]
pub const DEFAULT_BUFFER_BYTES: NonZeroUsize = unsafe {
    // SAFETY: `256 * 1024` is a non-zero constant.
    NonZeroUsize::new_unchecked(256 * 1024)
};
/// Default buffer size for `$ATTRIBUTE_LIST` and extension-record reads.
///
/// These reads are typically smaller and less sequential than the main `$MFT`
/// scan, so the attribute buffer is tuned independently from
/// [`DEFAULT_BUFFER_BYTES`].
#[allow(clippy::useless_nonzero_new_unchecked)]
pub const DEFAULT_ATTR_BUFFER_BYTES: NonZeroUsize = unsafe {
    // SAFETY: `16 * 1024` is a non-zero constant.
    NonZeroUsize::new_unchecked(16 * 1024)
};

/// Raw `$MFT` reader bound to an open [`Volume`].
pub struct RawMft<'a> {
    /// Volume whose `$MFT` is being read.
    volume: &'a Volume,
    /// Parsed NTFS boot-sector geometry.
    boot: BootSector,
    /// Shared extent map for the `$MFT` data stream.
    extent_map: Arc<ExtentMap>,
    /// Shared contents of the `$MFT` bitmap stream when available.
    bitmap: Arc<[u8]>,
}

impl<'a> RawMft<'a> {
    /// Create a volume reader using this MFT's discovered sector geometry.
    fn volume_reader(&self, buffer_bytes: NonZeroUsize) -> Result<VolumeReader, MftError> {
        VolumeReader::new(self.volume, self.boot.bytes_per_sector as u64, buffer_bytes)
    }

    /// Internal access to the bound volume for crate-level helpers.
    pub(crate) fn volume(&self) -> &'a Volume {
        self.volume
    }

    /// Total number of FILE records this MFT can address.
    #[must_use]
    #[inline]
    pub fn record_count(&self) -> u64 {
        self.extent_map.record_count()
    }

    /// Cluster size in bytes.
    #[must_use]
    #[inline]
    pub fn cluster_size(&self) -> u64 {
        self.boot.cluster_size
    }

    /// File record size in bytes.
    #[must_use]
    #[inline]
    pub fn file_record_size(&self) -> usize {
        self.boot.file_record_size as usize
    }

    /// Read a single record by number, including unused and extension records.
    ///
    /// Returns `Ok(None)` for a sparse hole or a rejected FILE header. A valid
    /// header with a bad update sequence array instead returns an error.
    /// Metadata uses the full collection options and selective, one-level
    /// enrichment. Enrichment failures are tolerated; inspect
    /// [`RawMftEntry::metadata`] for observed fields or use [`MftStream`] for
    /// per-slot errors and detailed completeness evidence.
    ///
    /// # Errors
    /// Returns I/O failures, invalid record numbers or extent arithmetic, and
    /// fixup validation errors such as [`MftError::FixupMismatch`].
    #[must_use = "the returned record is discarded if not inspected"]
    pub fn read_record(&self, number: u64) -> Result<Option<RawMftEntry>, MftError> {
        let mut reader = self.volume_reader(DEFAULT_BUFFER_BYTES)?;
        read_record_at(
            &mut reader,
            &self.boot,
            self.extent_map.as_ref(),
            number,
            EntryBuildOptions::full(),
        )
    }

    /// Cached allocation state from `$MFT::$BITMAP`.
    ///
    /// `Some(true)` is used, `Some(false)` is unused. `None` means the bitmap
    /// does not cover this slot, is unavailable, or the number is outside this
    /// reader's capacity. This snapshot can disagree with a live FILE header.
    #[must_use]
    pub fn bitmap_usage(&self, record_number: u64) -> Option<bool> {
        if record_number >= self.record_count() {
            return None;
        }
        let byte = usize::try_from(record_number / 8).ok()?;
        let bit = (record_number % 8) as u8;
        self.bitmap.get(byte).map(|b| (b >> bit) & 1 != 0)
    }

    /// Internal selection is conservative when allocation evidence is missing.
    pub(crate) fn bitmap_used(&self, record_number: u64) -> bool {
        let Ok(byte) = usize::try_from(record_number / 8) else {
            return true;
        };
        self.bitmap
            .get(byte)
            .is_none_or(|b| (b >> (record_number % 8)) & 1 != 0)
    }
}

impl std::fmt::Debug for RawMft<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RawMft")
            .field("volume", &self.volume)
            .field("record_count", &self.record_count())
            .field("file_record_size", &self.file_record_size())
            .field("bitmap_bytes", &self.bitmap.len())
            .finish_non_exhaustive()
    }
}

impl Volume {
    /// Open the raw `$MFT` reader for this volume.
    ///
    /// Convenience for [`RawMft::new`]. NTFS only — ReFS volumes return
    /// [`MftError::UnsupportedFilesystem`].
    pub fn raw_mft(&self) -> Result<RawMft<'_>, MftError> {
        RawMft::new(self)
    }
}
