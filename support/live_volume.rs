//! Explicit device selection shared by opt-in integration tests.
// Each test binary uses a different subset of these helpers.
#![allow(dead_code)]

#[cfg(windows)]
use ntfs_mft::MftError;
use ntfs_mft::{MftResult, Volume};
use std::{ffi::OsString, io};

pub fn required_source(value: Option<OsString>, variable: &str) -> MftResult<OsString> {
    value.filter(|s| !s.is_empty()).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("set {variable} explicitly before running this ignored live-device test"),
        )
        .into()
    })
}

pub fn configured_source(variable: &str) -> MftResult<OsString> {
    required_source(std::env::var_os(variable), variable)
}

#[cfg(windows)]
pub fn parse_drive_letter(source: &std::ffi::OsStr) -> MftResult<char> {
    match source.to_str().map(str::as_bytes) {
        Some([letter] | [letter, b':']) if letter.is_ascii_alphabetic() => {
            Ok((*letter as char).to_ascii_uppercase())
        }
        _ => Err(MftError::InvalidOptions(
            "live test drive must be a single letter such as C or C:",
        )),
    }
}

#[cfg(windows)]
pub fn drive_from_env(variable: &str) -> MftResult<char> {
    parse_drive_letter(&configured_source(variable)?)
}

pub fn open() -> MftResult<Volume> {
    #[cfg(windows)]
    {
        Volume::from_drive_letter(drive_from_env("NTFS_MFT_TEST_DRIVE")?)
    }
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        let source = std::path::PathBuf::from(configured_source("NTFS_MFT_TEST_VOLUME")?);
        if source.starts_with("/dev") {
            Volume::from_device_path(source)
        } else {
            Volume::from_mount_point(source)
        }
    }
}
