//! Raw `$MFT` reader for NTFS volumes.
//!
//! This module reads the `$MFT` file directly from the volume and parses
//! each FILE record to expose rich per-record metadata that the USN-based
//! the Windows FSCTL-based `Mft` enumerator cannot surface (full timestamps, real
//! and allocated size, hard link count, alternate data streams, sparse
//! / compressed flags, data-run summary, file-name namespace, etc.).
//!
//! Quick start:
//!
//! ```no_run
//! use ntfs_mft::{volume::Volume, raw_mft::RawMft};
//!
//! # #[cfg(windows)] {
//! let volume = Volume::from_drive_letter('C').expect("open volume");
//! let mft = RawMft::new(&volume).expect("read $MFT");
//! for entry in mft.try_iter().expect("iter") {
//!     match entry {
//!         Ok(e) if e.is_used => {
//!             println!("{:>8}: {}", e.record_number, e.file_name.to_string_lossy());
//!         }
//!         _ => {}
//!     }
//! }
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
mod diagnostics;
pub use diagnostics::{RawMftDiagnosticRecord, RawMftScanDiagnostic, RawMftScanReport};
// Enabled only by the non-publishable differential test package's build script.
#[cfg(all(test, ntfs_mft_differential))]
#[path = "../../differential-tests/tests/mod.rs"]
mod differential_tests;
mod entry_build;
pub mod history;
mod io;
mod layout;
mod offline;
mod options;
mod parallel;
mod path_resolver;
mod reader;
mod serial;
#[cfg(all(test, windows))]
mod tests;

use std::num::NonZeroUsize;
use std::sync::Arc;

use crate::{
    errors::MftError,
    raw_mft::{
        entry_build::EntryBuildOptions,
        io::VolumeReader,
        layout::{boot::BootSector, extent::ExtentMap},
        reader::read_record_at,
    },
    volume::Volume,
};

pub use byte_reader::MftByteReader;
pub use chunk_plan::{RawMftChunkPlanOptions, RawMftChunkPlanOptionsBuilder, RawMftWorkChunk};
pub use entry_build::{AdsInfo, RawMftBatchEntry, RawMftChunkBatch, RawMftEntry, RawMftLink};
pub use layout::attribute::FileNameNamespace;
pub use layout::data_run::{DataRun as DataRunInfo, DataRunSummary};
pub use offline::{
    MftStream, MftStreamIter, MftStreamOptions, OfflineDiagnostic, OfflineExternalPayload,
    OfflineFieldState, OfflineIssue, OfflineMetadata, OfflineMftRecord,
};
/// Shared metadata presence vocabulary for raw diagnostic and offline records.
pub use offline::{OfflineFieldState as MftFieldState, OfflineMetadata as MftMetadata};
pub use options::{
    RawMftEntryOptions, RawMftLeanScanOptions, RawMftLeanScanOptionsBuilder, RawMftReadBuffers,
    RawMftRecordRange, RawMftScanOptions, RawMftScanOptionsBuilder,
};
pub use parallel::RawMftParallelScheduling;
pub use parallel::{Lean, RawMftParallelScan, Rich, Unselected};
pub use path_resolver::RawMftPathResolver;
pub use serial::{RawMftDiagnosticIter, RawMftIter};

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
    // SAFETY: `64 * 1024` is a non-zero constant.
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
    pub fn file_record_size(&self) -> u64 {
        self.boot.file_record_size
    }

    /// Create a [`RawMftPathResolver`] for entries produced by this raw `$MFT` reader.
    ///
    /// Construction scans all base records, including root record 5, and stores
    /// their selected names in memory: O(records + names) space and a full scan
    /// of I/O. Scan I/O errors are propagated; malformed records may still be
    /// skipped by the raw scanner. This is not an atomic snapshot. Build a
    /// [`crate::MftPathIndex`] during an existing scan and use
    /// [`RawMftPathResolver::from_index`] to avoid a second pass.
    ///
    /// Default lookups verify sequence numbers at every step. On Windows, call `RawMftPathResolver::with_live_fallback` if you
    /// explicitly want best-effort current-volume fallback for snapshot misses.
    pub fn path_resolver(&self) -> crate::MftResult<RawMftPathResolver<'a>> {
        RawMftPathResolver::new(self)
    }

    /// Read a single record by number. Returns `Ok(None)` when the
    /// record falls in a sparse hole or does not contain a valid FILE record.
    #[must_use = "the returned record is discarded if not inspected"]
    pub fn read_record(&self, number: u64) -> Result<Option<RawMftEntry>, MftError> {
        let mut reader = VolumeReader::new(self.volume, self.boot.bytes_per_sector as u64)?;
        read_record_at(
            &mut reader,
            &self.boot,
            self.extent_map.as_ref(),
            number,
            EntryBuildOptions::full(),
        )
    }

    /// True if `record_number` is marked as in-use in the `$BITMAP`.
    /// Returns `true` when no bitmap is available.
    #[must_use]
    pub fn bitmap_used(&self, record_number: u64) -> bool {
        if self.bitmap.is_empty() {
            return true;
        }
        let byte = (record_number / 8) as usize;
        let bit = (record_number % 8) as u8;
        match self.bitmap.get(byte) {
            Some(&b) => (b >> bit) & 1 != 0,
            None => false,
        }
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

#[cfg(test)]
mod fixture_tests;

#[cfg(test)]
mod esd_iso_tests;
