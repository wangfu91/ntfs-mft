//! Shared environment-based volume opening for benchmarks and profiling tools.

use std::env;

use ntfs_mft::{errors::MftError, volume::Volume};

/// Read the Windows drive letter selected by the benchmark environment.
pub fn pick_drive() -> char {
    env::var("NTFS_MFT_TEST_DRIVE")
        .ok()
        .and_then(|value| value.chars().next())
        .map(|drive| drive.to_ascii_uppercase())
        .unwrap_or('C')
}

/// Open the volume selected by the benchmark environment.
pub fn open_volume(drive: char) -> Result<Volume, MftError> {
    #[cfg(windows)]
    {
        Volume::from_drive_letter(drive)
    }
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        let _ = drive;
        let source = env::var("NTFS_MFT_TEST_VOLUME").map_err(|_| {
            MftError::InvalidMountPoint(
                "set NTFS_MFT_TEST_VOLUME to an NTFS mount or device".into(),
            )
        })?;
        if source.starts_with("/dev/") {
            Volume::from_device_path(source)
        } else {
            Volume::from_mount_point(source)
        }
    }
}
