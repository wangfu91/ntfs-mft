//! Real boot sector, MFT and fragmented MFT bitmap; no device permissions needed.
use super::super::{
    entry_build::{EntryBuildOptions, RawMftEntry},
    layout::record::FileRecord,
};
use crate::{
    MftResult, RawMftChunkPlanOptions, RawMftParallelScheduling, RawMftScanOptions,
    volume::sparse_fixture::SparseVolumeFixture,
};
use std::{
    io::{Read, Seek, SeekFrom},
    num::{NonZeroU64, NonZeroUsize},
    sync::Arc,
};

const BOOT: &[u8] = include_bytes!("../../../tests/fixtures/sparse-volume-fixture/boot.bin");
const MFT: &[u8] = include_bytes!("../../../tests/fixtures/sparse-volume-fixture/mft.bin");
const BITMAP0: &[u8] =
    include_bytes!("../../../tests/fixtures/sparse-volume-fixture/mft-bitmap-0.bin");
const BITMAP1: &[u8] =
    include_bytes!("../../../tests/fixtures/sparse-volume-fixture/mft-bitmap-1.bin");

fn source(with_bitmap_tail: bool) -> Arc<SparseVolumeFixture> {
    let mut ranges = vec![
        (0, BOOT.to_vec()),
        (3_221_225_472, MFT.to_vec()),
        (3_221_221_376, BITMAP0.to_vec()),
    ];
    if with_bitmap_tail {
        ranges.push((151_552, BITMAP1.to_vec()));
    }
    Arc::new(SparseVolumeFixture::new(ranges))
}

#[test]
fn sparse_fixture_bootstrap_reads_both_bitmap_extents() -> MftResult<()> {
    let volume = source(true).open()?;
    let raw = volume.raw_mft()?;
    assert_eq!(raw.cluster_size(), 4096);
    assert_eq!(raw.file_record_size(), 1024);
    assert_eq!(raw.record_count(), 1280);
    assert_eq!(&*raw.bitmap, [BITMAP0, BITMAP1].concat());
    assert_eq!((0..1280).filter(|&n| raw.bitmap_used(n)).count(), 1116);
    // Even an all-zero logical tail must be supplied by the sparse fixture.
    assert!(source(false).open()?.raw_mft().is_err());
    assert!(source(true).read_at(&mut [0; 1], 512).is_err());
    Ok(())
}

#[test]
fn sparse_fixture_serial_matches_both_parallel_schedulers() -> Result<(), Box<dyn std::error::Error>>
{
    let volume = source(true).open()?;
    let raw = volume.raw_mft()?;
    let options = RawMftScanOptions::builder().start_record(0).build();
    let serial = raw
        .scan_entries_with_options(options.clone())?
        .collect::<MftResult<Vec<_>>>()?;
    assert_eq!(serial.len(), 1116);
    assert_eq!(serial.iter().filter(|e| e.is_directory).count(), 107);
    let chunks = raw.plan_chunks_with_options(
        RawMftChunkPlanOptions::builder()
            .max_records_per_chunk(NonZeroU64::new(16).unwrap())
            .build(),
        &options,
    );
    assert!(chunks.len() > 1);
    for scheduling in [
        RawMftParallelScheduling::Dynamic,
        RawMftParallelScheduling::Contiguous,
    ] {
        let rich = raw
            .parallel()
            .rich()
            .scan_options(options.clone())
            .chunk_plan(
                RawMftChunkPlanOptions::builder()
                    .max_records_per_chunk(NonZeroU64::new(16).unwrap())
                    .build(),
            )
            .workers(NonZeroUsize::new(4).unwrap())
            .scheduling(scheduling);
        assert_eq!(
            rich.clone()
                .collect_batches()?
                .into_iter()
                .flat_map(|b| b.entries)
                .collect::<Vec<_>>(),
            serial
        );
        let mut folded = Vec::new();
        rich.try_fold_chunks(
            |_| Vec::new(),
            |entries, entry| {
                entries.push(entry);
                Ok::<_, std::convert::Infallible>(())
            },
            |entries| {
                folded.extend(entries);
                Ok::<_, std::convert::Infallible>(())
            },
        )?;
        assert_eq!(folded, serial);
        let batches = raw
            .parallel()
            .lean()
            .chunks(chunks.clone())
            .scan_options(options.clone().into_lean())
            .workers(NonZeroUsize::new(4).unwrap())
            .scheduling(scheduling)
            .collect_batches()?;
        let actual = batches
            .into_iter()
            .flat_map(|b| b.entries)
            .collect::<Vec<_>>();
        assert_eq!(
            actual,
            serial
                .iter()
                .cloned()
                .map(crate::RawMftBatchEntry::from)
                .collect::<Vec<_>>()
        );
    }
    Ok(())
}

#[test]
fn sparse_fixture_owned_stream_preserves_all_original_bytes() -> MftResult<()> {
    let volume = source(true).open()?;
    let raw = volume.raw_mft()?;
    let mut first = raw.byte_reader()?;
    let mut second = raw.byte_reader()?;
    second.seek(SeekFrom::Start(5 * 1024))?;
    let mut signature = [0; 4];
    second.read_exact(&mut signature)?;
    assert_eq!(&signature, b"FILE");
    drop(second);
    drop(raw);
    drop(volume);
    let mut bytes = Vec::new();
    first.read_to_end(&mut bytes)?;
    assert_eq!(bytes, MFT);
    first.seek(SeekFrom::Start(256 * 1024 - 3))?;
    let mut boundary = [0; 16];
    first.read_exact(&mut boundary)?;
    assert_eq!(boundary, MFT[256 * 1024 - 3..256 * 1024 + 13]);
    Ok(())
}

#[test]
fn sparse_fixture_record_inventory_matches_independent_decoder() -> MftResult<()> {
    let (mut files, mut other, mut dirs) = (0, 0, 0);
    for (number, bytes) in MFT.as_chunks::<1024>().0.iter().enumerate() {
        let mut bytes = bytes.to_vec();
        if &bytes[..4] != b"FILE" {
            other += 1;
            assert!(FileRecord::parse(number as u64, None, &mut bytes).is_err());
            continue;
        }
        let record = FileRecord::parse(number as u64, None, &mut bytes)?;
        let entry = RawMftEntry::from_record_with_attr_list(&record, EntryBuildOptions::full()).0;
        assert!(entry.is_used);
        assert_eq!(entry.base_record_number, 0);
        files += 1;
        dirs += usize::from(entry.is_directory);
    }
    assert_eq!((files, other, dirs), (1116, 164, 107));
    Ok(())
}

#[test]
fn sparse_fixture_known_files_match_independently_decoded_text_fixture() -> MftResult<()> {
    let expected: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/sparse-volume-fixture/expected.json"
    ))
    .unwrap();
    let volume = source(true).open()?;
    let raw = volume.raw_mft()?;
    for item in expected.as_array().unwrap() {
        let number = item["record"].as_u64().unwrap();
        let entry = raw.read_record(number)?.unwrap();
        assert_eq!(entry.file_name, item["name"].as_str().unwrap());
        assert_eq!(
            u64::from(entry.sequence_number),
            item["sequence"].as_u64().unwrap()
        );
        let reference = item["parent_reference"].as_u64().unwrap();
        assert_eq!(
            entry.parent_reference,
            crate::NtfsFileReference::new(reference)
        );
        assert_eq!(entry.is_directory, item["directory"].as_bool().unwrap());
        assert_eq!(entry.has_unnamed_data, !item["data_size"].is_null());
        if let Some(size) = item["data_size"].as_u64() {
            assert_eq!(entry.real_size, size);
        }
    }
    Ok(())
}

#[test]
fn stream_from_raw_infers_geometry_and_owns_its_handle() -> MftResult<()> {
    let volume = source(true).open()?;
    let raw = volume.raw_mft()?;
    let root = raw.read_record(5)?.unwrap();
    let mut stream = crate::MftStream::from_raw_mft(&raw)?;
    assert_eq!(stream.file_record_size(), raw.file_record_size());
    drop(raw);
    drop(volume);
    let parsed = stream.read_record(5)?;
    assert_eq!(parsed.entry.file_reference, root.file_reference);
    assert_eq!(parsed.entry.file_name, root.file_name);
    Ok(())
}
