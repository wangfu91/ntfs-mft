//! Live-volume smoke test; deterministic fragmented-device tests live in the module.
use ntfs_mft::{MftError, MftResult, MftStream, MftStreamOptions};
use std::io::{Read, Seek, SeekFrom};

#[path = "../support/live_volume.rs"]
mod live_volume;

#[test]
#[ignore = "requires an explicitly configured NTFS volume and raw-device permissions"]
fn volume_byte_reader_feeds_iterator_and_owns_independent_handle() -> MftResult<()> {
    let volume = live_volume::open()?;
    let raw = volume.raw_mft()?;
    eprintln!("Discovered raw MFT; reading root through existing API");
    let record_size = raw.file_record_size();
    let expected = raw
        .read_record(5)?
        .ok_or(MftError::MftAttributeMissing("root record"))?;
    eprintln!("Opening first MFT byte reader");
    let mut first = raw.byte_reader()?;
    let mut second = raw.byte_reader()?;
    assert!(first.len() <= raw.record_count() * record_size as u64);
    assert!(!first.is_empty());
    let length = first.len();
    assert_eq!(first.seek(SeekFrom::End(0))?, length);
    assert_eq!(first.read(&mut [0; 4])?, 0);
    first.seek(SeekFrom::Start(0))?;
    second.seek(SeekFrom::Start(record_size as u64))?;
    let mut signature = [0; 4];
    second.read_exact(&mut signature)?;
    assert_eq!(&signature, b"FILE");
    first.read_exact(&mut signature)?;
    assert_eq!(&signature, b"FILE");
    drop(raw);
    drop(volume);
    drop(second);
    let mut parser = MftStream::from_reader(first, MftStreamOptions::new(record_size)?)?;
    assert_eq!(parser.record_count(), length / record_size as u64);
    let root = parser.read_record(5)?;
    assert_eq!(root.entry.file_name, expected.file_name);
    assert_eq!(root.entry.file_reference, expected.file_reference);
    assert!(root.entry.is_directory);
    let mut valid = 0;
    for result in parser.iter_records().take(32) {
        match result {
            Ok(_) => valid += 1,
            Err(error) if error.is_parse_error() => {}
            Err(error) => return Err(error),
        }
    }
    assert!(valid >= 12);
    eprintln!(
        "Validated live MFT byte reader: {length} bytes, {record_size}-byte records, {valid} valid records in first 32 slots"
    );
    Ok(())
}
