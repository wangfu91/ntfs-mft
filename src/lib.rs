#![deny(missing_docs)]
//! Read-only NTFS Master File Table scanning on Windows, Linux, and macOS.
//!
//! A **live volume scan** reads directly from an NTFS volume/device through
//! [`RawMft`]. An **offline MFT scan** parses an extracted file or buffer.
//! See [`raw_mft`] for scan options, parallel iteration, and limitations.
//! [`MftStream`] parses a logical `$MFT` stream from either source; its type does
//! not imply offline provenance or a stable snapshot.
//! **Read** denotes one record or a byte operation; **scan** denotes traversal.
//!
//! | API | Projection | Default selection | Parse failures |
//! | --- | --- | --- | --- |
//! | [`RawMft::scan_entries`] | Rich entries | `24..`, bitmap-used base records | Skipped |
//! | [`RawMft::parallel`] then `.rich()` | Rich batches | Same raw selection | Skipped |
//! | [`RawMft::parallel`] then `.lean()` | Lean batches | Same raw selection | Skipped |
//! | [`MftStream::iter_records`] | Stream records and evidence | Every slot from zero | Returned per slot |
//!
//! Scan I/O errors are returned. Stream iterators stop on I/O failure; parse
//! errors can be recoverable. No live scan is an atomic snapshot.
//! Rich entries retain their [`RawMftEntryOptions`]; lean options omit metadata
//! settings for fields they cannot return. Empty ordinary fields do not prove
//! absence: use [`MftStream`] and retain its metadata when completeness matters.
//!
//! [`NtfsName`] preserves exact UTF-16 on every platform; display and native-path
//! conversions are explicit. [`NtfsFileReference`] exposes checked NTFS identity.
//! Full-path reconstruction and directory-tree policy belong to the caller.
//! For a logical stream from an existing volume, [`MftStream::from_raw_mft`]
//! derives record size and opens an independently owned byte reader.
//!
//! # Quick start
//!
//! ```no_run
//! use ntfs_mft::{MftResult, Volume};
//! # fn main() -> MftResult<()> {
//! #[cfg(windows)]
//! let volume = Volume::from_drive_letter('C')?;
//! #[cfg(target_os = "linux")]
//! let volume = Volume::from_mount_point("/media/user/windows")?;
//! #[cfg(target_os = "macos")]
//! let volume = Volume::from_mount_point("/Volumes/Windows")?;
//! let raw = volume.raw_mft()?;
//! for entry in raw.scan_entries()?.take(20) {
//!     println!("{}", entry?.file_name);
//! }
//! # Ok(())
//! # }
//! ```
//!
//! Raw volume access requires suitable OS privileges. For an extracted `$MFT`,
//! use [`MftStream::from_path`] with [`MftStreamOptions`], using ordinary file
//! permissions. See the [API guide](https://github.com/wangfu91/ntfs-mft/blob/master/docs/api-guide.md)
//! and [release notes](https://github.com/wangfu91/ntfs-mft/blob/master/CHANGELOG.md).

mod ntfs_name;
pub use ntfs_name::NtfsName;
pub mod errors;
mod file_attributes;
mod file_reference;
#[cfg(windows)]
mod privilege;
pub mod raw_mft;
mod time;
pub mod volume;
#[doc(inline)]
pub use errors::{FileReferenceError, MftError, ScanError};
pub use file_attributes::FileAttributes;
pub use file_reference::NtfsFileReference;
#[doc(inline)]
pub use raw_mft::{
    AdsInfo, DEFAULT_ATTR_BUFFER_BYTES, DEFAULT_BUFFER_BYTES, FileNameNamespace, Lean,
    MftByteReader, MftDiagnostic, MftDiagnosticKind, MftExternalPayload, MftFieldState,
    MftMetadata, MftRecord, MftRecordIter, MftStream, MftStreamOptions, RawMft, RawMftChunkBatch,
    RawMftChunkPlanOptions, RawMftChunkPlanOptionsBuilder, RawMftEntry, RawMftEntryOptions,
    RawMftEntryOptionsBuilder, RawMftEntryScan, RawMftLeanEntry, RawMftLeanScanOptions,
    RawMftLeanScanOptionsBuilder, RawMftLink, RawMftParallelScan, RawMftParallelScheduling,
    RawMftReadBuffers, RawMftRecordRange, RawMftScanOptions, RawMftScanOptionsBuilder,
    RawMftWorkChunk, Rich, Unselected,
};
pub use time::Filetime;
#[doc(inline)]
pub use volume::Volume;
/// Result of an NTFS MFT operation.
pub type MftResult<T> = std::result::Result<T, MftError>;
/// Common imports for raw MFT scanning.
pub mod prelude {
    pub use crate::{
        FileAttributes, Filetime, MftError, MftResult, MftStream, MftStreamOptions,
        NtfsFileReference, NtfsName, RawMft, RawMftEntry, RawMftScanOptions, Volume,
    };
}
