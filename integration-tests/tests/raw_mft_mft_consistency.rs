#![cfg(windows)]

//! Compare raw records with independently enumerated Windows V2 records.
//! Run explicitly on a configured NTFS volume with Administrator privileges.
use ntfs_mft::{MftResult, RawMft};
use ntfs_mft_integration_tests::windows::{Mft, Volume};

#[path = "../../support/live_volume.rs"]
mod live_volume;

#[test]
#[ignore = "requires NTFS_MFT_TEST_DRIVE and Administrator privileges"]
fn raw_mft_and_mft_api_record_parity_standard_ids() -> MftResult<()> {
    let drive = live_volume::drive_from_env("NTFS_MFT_TEST_DRIVE")?;
    let raw_volume = live_volume::open()?;
    let raw_mft = RawMft::new(&raw_volume)?;
    let volume = Volume::from_drive_letter(drive)?;
    let mft = Mft::new(&volume);
    const SAMPLE_LIMIT: usize = 500;
    let mut samples = Vec::new();
    for item in mft.try_iter()? {
        // An enumeration/decoder failure must fail the test, not disappear in flatten().
        let entry = item?;
        if !entry.file_name.is_empty() {
            samples.push(entry);
            if samples.len() == SAMPLE_LIMIT {
                break;
            }
        }
    }
    assert!(
        !samples.is_empty(),
        "no named records available for parity comparison"
    );
    let mut compared = 0;
    let mut agreements = 0;
    let mut transient_misses = 0;
    for entry in &samples {
        let number = entry
            .fid
            .record_number()
            .expect("V2 must return standard NTFS IDs");
        let Some(raw_entry) = raw_mft.read_record(number)? else {
            transient_misses += 1;
            continue;
        };
        if !raw_entry.is_used {
            transient_misses += 1;
            continue;
        }
        compared += 1;
        let same_fid = Some(raw_entry.file_reference.get()) == entry.fid.as_u64();
        let same_parent = Some(raw_entry.parent_reference.get()) == entry.parent_fid.as_u64();
        let same_dir = raw_entry.is_directory == entry.is_dir();
        let same_name = raw_entry
            .file_name
            .to_string_lossy()
            .eq_ignore_ascii_case(&entry.file_name.to_string_lossy());
        if same_fid && same_parent && same_dir && same_name {
            agreements += 1;
        } else if compared - agreements <= 10 {
            eprintln!(
                "record {number}: fid={same_fid} parent={same_parent} directory={same_dir} name={same_name}"
            );
        }
    }
    assert!(
        compared >= samples.len().min(200),
        "insufficient comparable records: {compared}, transient misses: {transient_misses}"
    );
    // Allow up to 5% disagreement from concurrent filesystem changes.
    let rate = agreements as f64 / compared as f64;
    assert!(
        rate >= 0.95,
        "expected >=95% identity/metadata agreement: {agreements}/{compared}, transient misses: {transient_misses}"
    );
    eprintln!("Live parity: {agreements}/{compared} matched; transient misses: {transient_misses}");
    Ok(())
}
