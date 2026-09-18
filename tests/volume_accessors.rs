#![cfg(windows)]

//! Integration tests for the Volume::raw_mft convenience accessor.
//!
//! Each accessor must be equivalent to the explicit constructor it wraps.
//! Tests run against the `NTFS_MFT_TEST_DRIVE` volume (default `C`) and skip
//! gracefully when the process is not elevated or the drive is unavailable.

use ntfs_mft::{errors::MftError, raw_mft::RawMft, volume::Volume};

/// Pick the NTFS test volume, honoring `NTFS_MFT_TEST_DRIVE` (default `C`).
fn pick_drive() -> char {
    std::env::var("NTFS_MFT_TEST_DRIVE")
        .ok()
        .and_then(|s| s.chars().next())
        .map(|c| c.to_ascii_uppercase())
        .unwrap_or('C')
}

/// Open the test volume, or return `None` with a skip message.
fn open_test_volume(test_name: &str) -> Option<Volume> {
    let drive = pick_drive();
    match Volume::from_drive_letter(drive) {
        Ok(volume) => Some(volume),
        Err(MftError::NotElevated) => {
            eprintln!("{test_name}: skipping (requires admin privileges)");
            None
        }
        Err(error) => {
            eprintln!("{test_name}: skipping on {drive}: {error}");
            None
        }
    }
}

#[test]
fn raw_mft_accessor_matches_constructor_geometry() {
    let Some(volume) = open_test_volume("raw_mft_accessor_matches_constructor_geometry") else {
        return;
    };

    let via_accessor = match volume.raw_mft() {
        Ok(m) => m,
        Err(MftError::UnsupportedFilesystem(msg)) => {
            eprintln!("raw_mft_accessor_matches_constructor_geometry: skipping (not NTFS: {msg})");
            return;
        }
        Err(e) => panic!("volume.raw_mft() failed unexpectedly: {e}"),
    };
    let via_constructor = RawMft::new(&volume).expect("RawMft::new should succeed on NTFS");

    // Cluster size and file-record size come straight from the boot sector,
    // so they are fixed volume geometry and must match exactly.
    assert_eq!(
        via_accessor.cluster_size(),
        via_constructor.cluster_size(),
        "cluster_size must match between accessor and constructor"
    );
    assert_eq!(
        via_accessor.file_record_size(),
        via_constructor.file_record_size(),
        "file_record_size must match between accessor and constructor"
    );
    assert!(
        via_accessor.cluster_size() > 0 && via_accessor.file_record_size() > 0,
        "raw MFT geometry values must be non-zero"
    );
}
