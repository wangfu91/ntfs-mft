#![cfg(windows)]

//! Explicit ReFS rejection check. The selected volume must actually be ReFS.
use ntfs_mft::{MftError, MftResult, RawMft, Volume};
use windows::{Win32::Storage::FileSystem::GetVolumeInformationW, core::HSTRING};

#[path = "../support/live_volume.rs"]
mod live_volume;

#[test]
#[ignore = "requires NTFS_MFT_REFS_TEST_DRIVE and Administrator privileges"]
fn refs_volume_returns_unsupported_filesystem() -> MftResult<()> {
    let drive = live_volume::drive_from_env("NTFS_MFT_REFS_TEST_DRIVE")?;
    let root = HSTRING::from(format!("{drive}:\\"));
    let mut filesystem = [0u16; 32];
    // SAFETY: root is terminated by HSTRING, and the output slice is writable
    // for the duration of this synchronous call. No raw pointers are retained.
    unsafe {
        GetVolumeInformationW(&root, None, None, None, None, Some(&mut filesystem))?;
    }
    let length = filesystem
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(filesystem.len());
    assert_eq!(
        String::from_utf16_lossy(&filesystem[..length]),
        "ReFS",
        "selected test volume must be ReFS"
    );
    let volume = Volume::from_drive_letter(drive)?;
    assert!(matches!(
        RawMft::new(&volume),
        Err(MftError::UnsupportedFilesystem(_))
    ));
    Ok(())
}
