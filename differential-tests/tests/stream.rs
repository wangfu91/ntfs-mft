//! Exercise the production extent adapter and public offline parser together.
use super::*;
use crate::raw_mft::{
    byte_reader::MappedReader,
    layout::{data_run::DataRun, extent::ExtentMap},
};
use crate::{MftStream, MftStreamOptions};
use std::io::{Cursor, Read, Seek, SeekFrom};

fn scattered() -> TestResult<MappedReader<Cursor<Vec<u8>>>> {
    assert_eq!(MFT.len(), 13_402_112, "hydrate the pinned Git LFS fixture");
    let sectors = MFT.len() / 512;
    let mut disk = vec![0; MFT.len()];
    let mut runs = Vec::with_capacity(sectors);
    for (logical, bytes) in MFT.as_chunks::<512>().0.iter().enumerate() {
        let physical = sectors - 1 - logical;
        disk[physical * 512..(physical + 1) * 512].copy_from_slice(bytes);
        runs.push(DataRun::Data {
            lcn: physical as u64,
            clusters: 1,
        });
    }
    let map = ExtentMap::from_runs(&runs, 512, RECORD_SIZE as u64);
    Ok(MappedReader::new(Cursor::new(disk), &map)?)
}

#[test]
fn fragmented_stream_matches_pinned_parser_and_public_iterator() -> TestResult {
    let mut bytes_reader = scattered()?;
    let mut parser = MftStream::from_reader(scattered()?, MftStreamOptions::new(RECORD_SIZE)?)?;
    let mut iterator = parser.iter_records();
    let mut buffer = vec![0; RECORD_SIZE];
    let (mut files, mut zeros) = (0, 0);
    for (number, original) in MFT.as_chunks::<RECORD_SIZE>().0.iter().enumerate() {
        bytes_reader.read_exact(&mut buffer)?;
        assert_eq!(
            buffer, original,
            "slot {number}: original USA bytes through extent map"
        );
        let result = iterator.next().ok_or("public iterator ended early")?;
        if original[..4] == [0; 4] {
            assert!(result.is_err(), "slot {number}: zero slot must be an error");
            let other = MftEntry::from_buffer(original.to_vec(), number as u64)?;
            assert!(!other.header.is_valid());
            assert_eq!(other.valid_fixup, None);
            zeros += 1;
            continue;
        }
        // Full low-level SI/FN/DATA comparison retains the existing documented
        // timestamp normalization and all attributes, including continuations.
        compare(number as u64, &buffer)?;
        let reference = upstream(number as u64, original)?;
        let row = result?;
        let entry = row.entry;
        assert_eq!(row.stream_offset, number as u64 * RECORD_SIZE as u64);
        assert_eq!(entry.record_number, reference.number);
        assert_eq!(entry.sequence_number, reference.sequence);
        assert_eq!(
            entry.base_record_number,
            reference.base & 0x0000_ffff_ffff_ffff
        );
        assert_eq!(entry.flags & 0xf, reference.flags);
        assert_eq!(entry.hard_link_count, reference.links);
        for attr in &reference.attributes {
            if let Value::StandardInfo(times) = &attr.value {
                let actual = [
                    entry.si_created,
                    entry.si_modified,
                    entry.si_mft_modified,
                    entry.si_accessed,
                ]
                .map(|t| t.raw() / 10 * 10);
                assert_eq!(&actual, times, "slot {number}: public SI");
            }
        }
        if !entry.file_name.is_empty() {
            // Selection policies differ; require the selected tuple to be an
            // actual upstream FN attribute, without adopting upstream ranking.
            assert!(
                reference.attributes.iter().any(|attr| match &attr.value {
                    Value::FileName {
                        parent,
                        namespace,
                        name,
                        times,
                    } => {
                        let bytes: Vec<u8> = name.iter().flat_map(|u| u.to_le_bytes()).collect();
                        let decoded =
                            crate::raw_mft::layout::attribute::osstring_from_utf16le(&bytes);
                        *parent == entry.parent_reference.get()
                            && *namespace == entry.namespace as u8
                            && decoded.as_ref() == Some(&entry.file_name)
                            && *times
                                == [
                                    entry.fn_created,
                                    entry.fn_modified,
                                    entry.fn_mft_modified,
                                    entry.fn_accessed,
                                ]
                                .map(|t| t.raw() / 10 * 10)
                    }
                    _ => false,
                }),
                "slot {number}: selected public FN must be an upstream attribute"
            );
        }
        let data = reference
            .attributes
            .iter()
            .find_map(|attr| match &attr.value {
                Value::Data {
                    name,
                    size,
                    nonresident,
                } if name.is_empty() && nonresident.as_ref().is_none_or(|h| h.first_vcn == 0) => {
                    Some((size, nonresident))
                }
                _ => None,
            });
        assert_eq!(
            entry.has_unnamed_data,
            data.is_some(),
            "slot {number}: unnamed DATA presence"
        );
        if let Some((size, nonresident)) = data {
            assert_eq!(entry.real_size, *size, "slot {number}: public DATA size");
            assert_eq!(entry.is_resident, nonresident.is_none());
            assert_eq!(
                entry.allocated_size,
                nonresident.as_ref().map_or(*size, |h| h.allocated)
            );
        }
        files += 1;
    }
    assert!(iterator.next().is_none());
    assert_eq!((files, zeros), (13_068, 20));
    // Reverse random reads on the same parser after complete iteration.
    for number in [13000, 5000, 42, 5, 0] {
        let expected = upstream(
            number,
            &MFT[number as usize * RECORD_SIZE..(number as usize + 1) * RECORD_SIZE],
        )?;
        assert_eq!(
            parser.read_record(number)?.entry.sequence_number,
            expected.sequence
        );
        bytes_reader.seek(SeekFrom::Start(number * RECORD_SIZE as u64 + 507))?;
        let mut crossing = [0; 17];
        bytes_reader.read_exact(&mut crossing)?;
        assert_eq!(
            &crossing,
            &MFT[number as usize * RECORD_SIZE + 507..number as usize * RECORD_SIZE + 524]
        );
    }
    eprintln!(
        "fragmented adapter + public iterator: {files} FILE records and {zeros} zero slots compared with pinned upstream"
    );
    Ok(())
}
