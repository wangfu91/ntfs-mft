//! Synthetic 512-byte clusters with a FILE record split across nonadjacent runs.
use crate::{
    MftResult, MftStream, RawMftChunkPlanOptions, RawMftScanOptions,
    volume::sparse_fixture::SparseVolumeFixture,
};
use std::{
    num::{NonZeroU64, NonZeroUsize},
    sync::Arc,
};

fn record(nonresident: bool) -> Vec<u8> {
    let mut bytes = vec![0; 1024];
    bytes[..4].copy_from_slice(b"FILE");
    bytes[4..6].copy_from_slice(&48u16.to_le_bytes());
    bytes[6..8].copy_from_slice(&3u16.to_le_bytes());
    bytes[16..18].copy_from_slice(&1u16.to_le_bytes());
    bytes[20..22].copy_from_slice(&56u16.to_le_bytes());
    bytes[22..24].copy_from_slice(&1u16.to_le_bytes());
    bytes[28..32].copy_from_slice(&1024u32.to_le_bytes());
    for offset in [48, 510, 1022] {
        bytes[offset..offset + 2].copy_from_slice(&0xa55au16.to_le_bytes());
    }
    bytes[56..60].copy_from_slice(&0x80u32.to_le_bytes());
    let end = if nonresident {
        bytes[60..64].copy_from_slice(&72u32.to_le_bytes());
        bytes[64] = 1;
        bytes[80..88].copy_from_slice(&3u64.to_le_bytes());
        bytes[88..90].copy_from_slice(&64u16.to_le_bytes());
        for offset in [96, 104, 112] {
            bytes[offset..offset + 8].copy_from_slice(&2048u64.to_le_bytes());
        }
        bytes[120..127].copy_from_slice(&[0x11, 3, 2, 0x11, 1, 8, 0]);
        128
    } else {
        bytes[60..64].copy_from_slice(&32u32.to_le_bytes());
        bytes[72..76].copy_from_slice(&4u32.to_le_bytes());
        bytes[76..78].copy_from_slice(&24u16.to_le_bytes());
        bytes[80..84].copy_from_slice(b"test");
        88
    };
    bytes[end..end + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    bytes[24..28].copy_from_slice(&((end + 8) as u32).to_le_bytes());
    bytes
}

#[test]
fn bootstrap_reads_extension_record_across_nonadjacent_extents() -> MftResult<()> {
    let mut boot = vec![0; 512];
    boot[3..11].copy_from_slice(b"NTFS    ");
    boot[11..13].copy_from_slice(&512u16.to_le_bytes());
    boot[13] = 1;
    boot[48..56].copy_from_slice(&2u64.to_le_bytes());
    boot[64] = (-10i8) as u8;
    let mut first = record(true);
    for offset in [96, 104, 112] {
        first[offset..offset + 8].copy_from_slice(&3072u64.to_le_bytes());
    }
    // Resident ATTRIBUTE_LIST names the continuation at VCN 4 in record 1.
    first[128..132].copy_from_slice(&0x20u32.to_le_bytes());
    first[132..136].copy_from_slice(&56u32.to_le_bytes());
    first[144..148].copy_from_slice(&32u32.to_le_bytes());
    first[148..150].copy_from_slice(&24u16.to_le_bytes());
    first[152..156].copy_from_slice(&0x80u32.to_le_bytes());
    first[156..158].copy_from_slice(&32u16.to_le_bytes());
    first[160..168].copy_from_slice(&4u64.to_le_bytes());
    first[168..176].copy_from_slice(&((1u64 << 48) | 1).to_le_bytes());
    first[176..178].copy_from_slice(&2u16.to_le_bytes());
    first[184..188].copy_from_slice(&u32::MAX.to_le_bytes());
    first[24..28].copy_from_slice(&192u32.to_le_bytes());
    let mut extension = record(true);
    extension[32..40].copy_from_slice(&(1u64 << 48).to_le_bytes());
    extension[70..72].copy_from_slice(&2u16.to_le_bytes());
    extension[72..80].copy_from_slice(&4u64.to_le_bytes());
    extension[80..88].copy_from_slice(&5u64.to_le_bytes());
    extension[120..128].copy_from_slice(&[0x11, 2, 20, 0, 0, 0, 0, 0]);
    let source = Arc::new(SparseVolumeFixture::new(vec![
        (0, boot),
        (1024, [first.as_slice(), &extension[..512]].concat()),
        (5120, extension[512..].to_vec()),
    ]));
    let volume = source.open()?;
    assert_eq!(volume.raw_mft()?.record_count(), 3);
    Ok(())
}

#[test]
fn split_record_matches_stream_single_serial_and_parallel_reads() -> MftResult<()> {
    let mut boot = vec![0; 512];
    boot[3..11].copy_from_slice(b"NTFS    ");
    boot[11..13].copy_from_slice(&512u16.to_le_bytes());
    boot[13] = 1;
    boot[48..56].copy_from_slice(&2u64.to_le_bytes());
    boot[64] = (-10i8) as u8;
    let first = record(true);
    let second = record(false);
    let source = Arc::new(SparseVolumeFixture::new(vec![
        (0, boot),
        (1024, [first.as_slice(), &second[..512]].concat()),
        (5120, second[512..].to_vec()),
    ]));
    let volume = source.open()?;
    let raw = volume.raw_mft()?;
    let mut stream = MftStream::from_raw_mft(&raw)?;
    let expected = vec![stream.read_record(0)?.entry, stream.read_record(1)?.entry];
    assert_eq!(expected[1].real_size, 4);
    assert_eq!(raw.read_record(1)?, Some(expected[1].clone()));
    let options = RawMftScanOptions::builder()
        .range(0..2)
        .include_unused_records(true)
        .build();
    assert_eq!(
        raw.scan_entries_with_options(options.clone())?
            .collect::<MftResult<Vec<_>>>()?,
        expected
    );
    let plan = RawMftChunkPlanOptions::builder()
        .max_records_per_chunk(NonZeroU64::new(1).unwrap())
        .build();
    let batches = raw
        .parallel()
        .rich()
        .scan_options(options.clone())
        .chunk_plan(plan.clone())
        .workers(NonZeroUsize::new(2).unwrap())
        .collect_batches()?;
    assert_eq!(
        batches
            .into_iter()
            .flat_map(|b| b.entries)
            .collect::<Vec<_>>(),
        expected
    );
    let batches = raw
        .parallel()
        .lean()
        .scan_options(options.into_lean())
        .chunk_plan(plan)
        .workers(NonZeroUsize::new(2).unwrap())
        .collect_batches()?;
    assert_eq!(
        batches
            .into_iter()
            .flat_map(|b| b.entries)
            .collect::<Vec<_>>(),
        expected
            .into_iter()
            .map(crate::RawMftLeanEntry::from)
            .collect::<Vec<_>>()
    );
    Ok(())
}
