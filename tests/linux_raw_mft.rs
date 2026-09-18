#![cfg(target_os = "linux")]

use ntfs_mft::{MftResult, raw_mft::RawMft};

#[path = "../support/live_volume.rs"]
mod live_volume;

#[test]
#[ignore = "requires NTFS_MFT_TEST_VOLUME and raw-device permissions"]
fn mounted_ntfs_raw_mft_smoke_test() -> MftResult<()> {
    let volume = live_volume::open()?;
    let raw_mft = RawMft::new(&volume).expect("parse NTFS boot sector and discover $MFT");
    let entries: Vec<_> = raw_mft
        .scan_entries()
        .expect("create raw $MFT iterator")
        .take(128)
        .collect::<Result<_, _>>()
        .expect("parse raw $MFT prefix");
    assert!(
        !entries.is_empty(),
        "raw $MFT prefix should contain records"
    );

    let first = &entries[0];
    let reread = raw_mft
        .read_record(first.record_number)
        .expect("reread first record")
        .expect("first record should remain addressable");
    assert_eq!(reread.file_reference, first.file_reference);
    Ok(())
}
