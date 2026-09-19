//! Expectations independently decoded from the fixture's raw little-endian bytes.
use super::super::{
    entry_build::{EntryBuildOptions, RawMftEntry},
    layout::record::FileRecord,
};
use crate::{Filetime, MftError, MftResult, NtfsFileReference};
const MFT: &[u8] = include_bytes!("../../../tests/fixtures/omerbenamram-mft/MFT");
const RECORD_SIZE: usize = 1024;

fn entry(number: usize) -> MftResult<RawMftEntry> {
    let mut bytes = MFT[number * RECORD_SIZE..(number + 1) * RECORD_SIZE].to_vec();
    let record = FileRecord::parse(number as u64, None, &mut bytes)?;
    Ok(RawMftEntry::from_record_with_attr_list(&record, EntryBuildOptions::full()).0)
}

#[test]
fn fixture_inventory_and_all_file_records_parse() -> MftResult<()> {
    assert_eq!(MFT.len(), 13_402_112);
    assert_eq!(MFT.len() % RECORD_SIZE, 0);
    let (mut files, mut empty, mut used, mut directories, mut extensions) = (0, 0, 0, 0, 0);
    for (number, bytes) in MFT.as_chunks::<RECORD_SIZE>().0.iter().enumerate() {
        if bytes[..4] == [0; 4] {
            empty += 1;
            let mut bytes = bytes.to_vec();
            assert!(FileRecord::parse(number as u64, None, &mut bytes).is_err());
            continue;
        }
        assert_eq!(&bytes[..4], b"FILE", "record {number}");
        let parsed = entry(number)?;
        files += 1;
        used += usize::from(parsed.is_used);
        directories += usize::from(parsed.is_directory);
        extensions += usize::from(parsed.base_reference().is_some());
        assert_eq!(parsed.record_number, number as u64);
    }
    assert_eq!(
        (files, empty, used, directories, extensions),
        (13_068, 20, 11_839, 981, 4)
    );
    Ok(())
}

#[test]
fn known_system_files_have_expected_metadata() -> MftResult<()> {
    for (number, name, sequence, size, allocated) in [
        (0, "$MFT", 1, 13_402_112, 13_402_112),
        (1, "$MFTMirr", 1, 4096, 4096),
        (2, "$LogFile", 2, 8_421_376, 8_421_376),
        (4, "$AttrDef", 4, 2560, 4096),
        (6, "$Bitmap", 6, 77_016, 77_824),
        (7, "$Boot", 7, 8192, 8192),
        (10, "$UpCase", 10, 131_072, 131_072),
    ] {
        let parsed = entry(number)?;
        assert_eq!(parsed.file_name, name);
        assert_eq!(parsed.sequence_number, sequence);
        assert_eq!(
            parsed.file_reference,
            NtfsFileReference::try_from_parts(number as u64, sequence).unwrap()
        );
        assert_eq!(
            parsed.parent_reference,
            NtfsFileReference::try_from_parts(5, 5).unwrap()
        );
        assert_eq!(parsed.real_size, size);
        assert_eq!(parsed.allocated_size, allocated);
        assert_eq!(parsed.si_created, Filetime::new(128_276_814_522_523_952));
        assert_eq!(parsed.si_file_attributes.bits(), 6);
        assert!(parsed.is_used && parsed.has_unnamed_data);
        assert!(!parsed.is_directory && !parsed.is_resident);
        // Single-name records store their link in the primary name/parent fields.
        assert!(parsed.links.is_empty());
    }
    Ok(())
}

#[test]
fn directory_parent_references_preserve_sequence_numbers() -> MftResult<()> {
    for (number, name, parent, parent_sequence) in [
        (5, ".", 5, 5),
        (11, "$Extend", 5, 5),
        (28, "WINDOWS", 5, 5),
        (29, "system32", 28, 1),
        (30, "config", 29, 1),
    ] {
        let parsed = entry(number)?;
        assert_eq!(parsed.file_name, name);
        assert!(parsed.is_directory && parsed.is_used);
        assert_eq!(
            parsed.parent_reference,
            NtfsFileReference::try_from_parts(parent, parent_sequence).unwrap()
        );
        assert!(!parsed.has_unnamed_data);
    }
    Ok(())
}

#[test]
fn resident_data_and_named_nonresident_streams_are_distinct() -> MftResult<()> {
    let volume = entry(3)?;
    assert_eq!(volume.file_name, "$Volume");
    assert!(volume.has_unnamed_data && volume.is_resident);
    assert_eq!(volume.real_size, 0);
    let bad = entry(8)?;
    assert_eq!(bad.file_name, "$BadClus");
    assert!(bad.has_unnamed_data && bad.is_resident);
    assert_eq!(bad.real_size, 0);
    assert_eq!(bad.alternate_data_streams.len(), 1);
    let stream = &bad.alternate_data_streams[0];
    assert_eq!(stream.name, "$Bad");
    assert_eq!(stream.real_size, 1_261_821_952);
    assert_eq!(stream.allocated_size, 1_261_821_952);
    assert!(!stream.is_resident);
    Ok(())
}

#[test]
fn real_record_fixups_restore_trailers_and_reject_torn_sectors() -> MftResult<()> {
    let original = &MFT[..RECORD_SIZE];
    let usa = u16::from_le_bytes([original[4], original[5]]) as usize;
    let mut fixed = original.to_vec();
    FileRecord::parse(0, None, &mut fixed)?;
    for sector in 1..=2 {
        let trailer = sector * 512 - 2;
        assert_eq!(
            &fixed[trailer..trailer + 2],
            &original[usa + sector * 2..usa + sector * 2 + 2]
        );
        let mut damaged = original.to_vec();
        damaged[trailer] ^= 1;
        assert!(matches!(
            FileRecord::parse(0, None, &mut damaged),
            Err(MftError::FixupMismatch { number: 0 })
        ));
    }
    for length in [0, 4, 42, 512, 1023] {
        let mut truncated = original[..length].to_vec();
        assert!(FileRecord::parse(0, None, &mut truncated).is_err());
    }
    Ok(())
}
