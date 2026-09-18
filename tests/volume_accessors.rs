#![cfg(windows)]

//! Integration tests for the Volume::raw_mft convenience accessor.
//!
//! Each accessor must be equivalent to the explicit constructor it wraps.
//! Run explicitly with NTFS_MFT_TEST_DRIVE and Administrator privileges.

use ntfs_mft::{MftResult, RawMft};

#[path = "../support/live_volume.rs"]
mod live_volume;

#[test]
#[ignore = "requires NTFS_MFT_TEST_DRIVE and Administrator privileges"]
fn raw_mft_accessor_matches_constructor_geometry() -> MftResult<()> {
    let volume = live_volume::open()?;
    assert_eq!(
        volume.drive_letter(),
        Some(live_volume::drive_from_env("NTFS_MFT_TEST_DRIVE")?)
    );
    assert!(volume.mount_point().is_none());
    let via_accessor = volume.raw_mft()?;
    let via_constructor = RawMft::new(&volume)?;

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
    Ok(())
}
