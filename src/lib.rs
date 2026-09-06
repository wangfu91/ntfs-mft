#![deny(missing_docs)]
//! Read-only raw NTFS Master File Table scanning on Windows and Linux.
//!
//! Open a [`Volume`] and construct a [`RawMft`] to iterate metadata records.
//! See [`raw_mft`] for scan options, parallel iteration, and limitations.
#[cfg(not(any(windows, target_os = "linux")))]
compile_error!("ntfs-mft supports only Windows and Linux targets");
mod display;
pub mod errors;
mod file_attributes;
mod path;
#[cfg(windows)]
mod privilege;
pub mod raw_mft;
mod time;
pub mod types;
pub mod volume;
pub use errors::MftError;
pub use raw_mft::*;
pub use time::Filetime;
pub use types::{Fid, FileAttributes, Usn};
pub use volume::Volume;
/// Result of an NTFS MFT operation.
pub type MftResult<T> = std::result::Result<T, MftError>;
/// Common imports for raw MFT scanning.
pub mod prelude {
    pub use crate::{
        Fid, FileAttributes, Filetime, MftError, MftResult, RawMft, RawMftEntry,
        RawMftPathResolver, RawMftScanOptions, Usn, Volume,
    };
}
