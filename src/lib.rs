#![deny(missing_docs)]
//! Read-only raw NTFS Master File Table scanning on Windows, Linux, and macOS.
//!
//! Open a [`Volume`] and construct a [`RawMft`] to iterate metadata records.
//! See [`raw_mft`] for scan options, parallel iteration, and limitations.
//! For an extracted `$MFT` file or memory buffer, use [`MftStream`] without a volume.
//!
//! | API | Projection | Default selection | Parse failures |
//! | --- | --- | --- | --- |
//! | [`RawMft::try_iter`] | Rich entries | `24..`, bitmap-used base records | Skipped |
//! | [`RawMft::parallel`] then `.rich()` | Rich batches | Same raw selection | Skipped |
//! | [`RawMft::parallel`] then `.lean()` | Lean batches | Same raw selection | Skipped |
//! | [`RawMft::try_iter_diagnostic`] | Rich entries and evidence | Supplied raw selection | Counted, bounded retention |
//! | [`MftStream::iter_records`] | Offline records and evidence | Every slot from zero | Returned per slot |
//!
//! Scan I/O errors are returned. Diagnostic and offline iterators stop on I/O
//! failure; parse errors can be recoverable. No live scan is an atomic snapshot.
//! Rich entries retain their [`RawMftEntryOptions`]; lean options omit metadata
//! settings for fields they cannot return. Empty ordinary fields do not prove
//! absence: retain diagnostic metadata when completeness matters.
//!
//! [`NtfsName`] preserves exact UTF-16 on every platform; display and native-path
//! conversions are explicit. [`NtfsFileReference`] exposes checked NTFS identity.
//! Build [`MftPathIndex`] during a scan to avoid the extra scan performed by
//! [`RawMft::path_resolver`]. Include the root and all needed parent records.
//! For a logical stream from an existing volume, [`MftStream::from_raw_mft`]
//! derives record size and opens an independently owned byte reader.
#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
compile_error!("ntfs-mft supports only Windows, Linux, and macOS targets");
mod display;
mod ntfs_name;
pub use ntfs_name::NtfsName;
pub mod errors;
#[cfg(feature = "serde")]
pub mod export;
mod file_attributes;
mod path;
pub use path::{MftPathIndex, MftPathError, MftPathQuality, MftPathResolution};
#[cfg(windows)]
mod privilege;
pub mod raw_mft;
mod time;
pub mod types;
pub mod volume;
pub use errors::{MftError, ScanError};
pub use raw_mft::*;
pub use time::Filetime;
pub use types::{Fid, NtfsFileReference, FileReferenceError, FileAttributes, Usn};
pub use volume::Volume;
/// Result of an NTFS MFT operation.
pub type MftResult<T> = std::result::Result<T, MftError>;
/// Common imports for raw MFT scanning.
pub mod prelude {
    pub use crate::{
        Fid, NtfsFileReference, FileAttributes, Filetime, MftError, MftResult, MftStream, MftStreamOptions, RawMft,
        NtfsName, RawMftEntry, RawMftPathResolver, RawMftScanOptions, Usn, Volume,
    };
}
