use ntfs_mft::{errors::MftError, volume::Volume};

pub fn open_raw_volume(source: Option<String>) -> Result<(Volume, String), MftError> {
    #[cfg(windows)]
    {
        let drive = match source.as_deref() {
            None => 'C',
            Some(source) => parse_drive_letter(source)?,
        };
        Ok((Volume::from_drive_letter(drive)?, format!("{drive}:")))
    }
    #[cfg(any(target_os = "linux", target_os = "macos"))]
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

#[cfg(windows)]
fn parse_drive_letter(source: &str) -> Result<char, MftError> {
    match source.as_bytes() {
        [letter] | [letter, b':'] if letter.is_ascii_alphabetic() => {
            Ok((*letter as char).to_ascii_uppercase())
        }
        _ => Err(MftError::InvalidOptions(
            "Windows volume source must be a drive letter such as C or C:",
        )),
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::parse_drive_letter;

    #[test]
    fn drive_syntax_accepts_only_drive_letters() {
        for source in ["C", "c:", "Z:"] {
            assert!(parse_drive_letter(source).is_ok());
        }
        for source in ["", "data", "C:\\snapshot.MFT", "C:/", "1", "é", " C"] {
            assert!(parse_drive_letter(source).is_err(), "accepted {source:?}");
        }
    }
}
