use ntfs_mft::{errors::MftError, volume::Volume};

pub fn open_raw_volume(source: Option<String>) -> Result<(Volume, String), MftError> {
    #[cfg(windows)]
    {
        let drive = source
            .as_deref()
            .and_then(|value| value.chars().next())
            .unwrap_or('C')
            .to_ascii_uppercase();
        Ok((Volume::from_drive_letter(drive)?, format!("{drive}:")))
    }
    #[cfg(target_os = "linux")]
    {
        let source = source.ok_or_else(|| {
            MftError::InvalidMountPoint("pass an NTFS mount point or device path".into())
        })?;
        let volume = if source.starts_with("/dev/") {
            Volume::from_device_path(&source)?
        } else {
            Volume::from_mount_point(&source)?
        };
        Ok((volume, source))
    }
}
