//! Live-volume smoke test; deterministic fragmented-device tests live in the module.
use ntfs_mft::{MftError, MftResult, MftStream, MftStreamOptions, Volume};
use std::io::{Read, Seek, SeekFrom};

#[test]
fn volume_byte_reader_feeds_iterator_and_owns_independent_handle() -> MftResult<()> {
    #[cfg(windows)]
    let opened = {
        let drive = std::env::var("NTFS_MFT_TEST_DRIVE")
            .ok()
            .and_then(|s| s.chars().next())
            .unwrap_or('C');
        eprintln!("Testing Windows volume {drive}:");
        Volume::from_drive_letter(drive)
    };
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    let opened = {
        let Ok(path) = std::env::var("NTFS_MFT_TEST_VOLUME") else {
            eprintln!("SKIP: set NTFS_MFT_TEST_VOLUME to an NTFS device or mount point");
            return Ok(());
        };
        if path.starts_with("/dev/") {
            Volume::from_device_path(path)
        } else {
            Volume::from_mount_point(path)
        }
    };
    let volume = match opened {
        Ok(volume) => volume,
        Err(error) => {
            eprintln!("SKIP: volume unavailable: {error}");
            return Ok(());
        }
    };
    eprintln!("Opened volume; discovering raw MFT");
    let raw = match volume.raw_mft() {
        Ok(raw) => raw,
        Err(MftError::UnsupportedFilesystem(reason)) => {
            eprintln!("SKIP: {reason}");
            return Ok(());
        }
        Err(error) => return Err(error),
    };
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
