use super::*;
use crate::{
    MftResult, Volume,
    raw_mft::layout::{boot::BootSector, data_run::DataRun, extent::ExtentMap},
    volume::captured::CapturedVolume,
};
use std::{num::NonZeroU64, sync::Arc};

// An independent, synthetic volume: every slot is a valid base FILE record,
// but only slots 2 and 26 are marked used. This leaves whole unused chunk bands.
fn volume() -> MftResult<Volume> {
    let mut bytes = vec![0; 32 * 1024];
    for (number, record) in bytes.as_chunks_mut::<1024>().0.iter_mut().enumerate() {
        record[..4].copy_from_slice(b"FILE");
        record[4..6].copy_from_slice(&48u16.to_le_bytes());
        record[6..8].copy_from_slice(&3u16.to_le_bytes());
        record[16..18].copy_from_slice(&1u16.to_le_bytes());
        record[20..22].copy_from_slice(&56u16.to_le_bytes());
        record[22..24].copy_from_slice(&u16::from(matches!(number, 2 | 26)).to_le_bytes());
        record[24..28].copy_from_slice(&60u32.to_le_bytes());
        record[28..32].copy_from_slice(&1024u32.to_le_bytes());
        record[48..50].copy_from_slice(&0xabcd_u16.to_le_bytes());
        record[56..60].copy_from_slice(&u32::MAX.to_le_bytes());
        record[510..512].copy_from_slice(&0xabcd_u16.to_le_bytes());
        record[1022..1024].copy_from_slice(&0xabcd_u16.to_le_bytes());
    }
    Arc::new(CapturedVolume::new(vec![(0, bytes)])).open()
}

fn mft(volume: &Volume) -> RawMft<'_> {
    RawMft {
        volume,
        boot: BootSector {
            bytes_per_sector: 512,
            mft_lcn: 0,
            file_record_size: 1024,
            cluster_size: 1024,
            mft_byte_offset: 0,
        },
        extent_map: Arc::new(ExtentMap::from_runs(
            &[DataRun::Data {
                lcn: 0,
                clusters: 32,
            }],
            1024,
            1024,
        )),
        bitmap: Arc::from([4, 0, 0, 4]),
    }
}

fn plan() -> RawMftChunkPlanOptions {
    RawMftChunkPlanOptions::builder()
        .max_records_per_chunk(NonZeroU64::new(4).unwrap())
        .build()
}

fn numbers(batches: Vec<RawMftChunkBatch>) -> Vec<u64> {
    batches
        .into_iter()
        .flat_map(|b| b.entries)
        .map(|e| e.record_number)
        .collect()
}

#[test]
fn automatic_selection_matches_serial_for_ranges_usage_and_builder_order() -> MftResult<()> {
    let volume = volume()?;
    let raw = mft(&volume);
    for (start, end, include_unused, expected) in [
        (0, Some(12), false, vec![2]),
        (0, Some(12), true, (0..12).collect()),
        (4, Some(12), false, vec![]),
        (4, Some(12), true, (4..12).collect()),
        (25, None, false, vec![26]),
        (30, Some(u64::MAX), true, vec![30, 31]),
        (8, Some(8), true, vec![]),
        (12, Some(4), true, vec![]),
        (40, None, true, vec![]),
    ] {
        let options = RawMftScanOptions::builder()
            .start_record(start)
            .end_record(end)
            .include_unused_records(include_unused)
            .build();
        let serial = raw
            .try_iter_with_options(options.clone())?
            .map(|e| e.map(|e| e.record_number))
            .collect::<MftResult<Vec<_>>>()?;
        assert_eq!(serial, expected);
        for workers in [1, 3] {
            for scheduling in [
                RawMftParallelScheduling::Dynamic,
                RawMftParallelScheduling::Contiguous,
            ] {
                let scan = raw
                    .parallel()
                    .workers(NonZeroUsize::new(workers).unwrap())
                    .scheduling(scheduling);
                let first = scan
                    .clone()
                    .scan_options(options.clone())
                    .chunk_plan(plan());
                let second = scan.chunk_plan(plan()).scan_options(options.clone());
                assert_eq!(
                    first.resolved_chunks()?,
                    raw.plan_chunks_with_options(plan(), &options)
                );
                assert_eq!(numbers(first.collect_batches()?), expected);
                assert_eq!(numbers(second.collect_batches()?), expected);
            }
        }
    }
    assert_eq!(numbers(raw.parallel().collect_batches()?), vec![26]);
    Ok(())
}

#[test]
fn explicit_chunks_intersect_selection_preserving_order() -> MftResult<()> {
    let volume = volume()?;
    let raw = mft(&volume);
    let chunks = [(10, 40), (0, 6), (6, 6), (9, 3), (6, 9), (40, u64::MAX)]
        .map(|(start_record, end_record)| RawMftWorkChunk {
            start_record,
            end_record,
        })
        .to_vec();
    let options = RawMftScanOptions::builder()
        .start_record(4)
        .end_record(Some(12))
        .include_unused_records(true)
        .build();
    let scan = raw
        .parallel()
        .chunks(chunks.clone())
        .scan_options(options.clone())
        .workers(NonZeroUsize::new(3).unwrap());
    let expected = vec![10, 11, 4, 5, 6, 7, 8];
    let batches = scan.clone().collect_batches()?;
    assert_eq!(
        batches
            .iter()
            .map(|b| (b.chunk.start_record, b.chunk.end_record))
            .collect::<Vec<_>>(),
        vec![(10, 12), (4, 6), (6, 9)]
    );
    assert_eq!(numbers(batches), expected);
    assert_eq!(
        numbers(
            raw.parallel()
                .scan_options(options)
                .chunks(chunks)
                .collect_batches()?
        ),
        expected
    );

    let mut visited = Vec::new();
    scan.clone().for_each_batch(|batch| {
        visited.extend(batch.entries.into_iter().map(|e| e.record_number));
        Ok(())
    })?;
    assert_eq!(visited, expected);
    let mut mapped = Vec::new();
    scan.clone().map_chunks(
        |batch| Ok(numbers(vec![batch])),
        |part| {
            mapped.extend(part);
            Ok(())
        },
    )?;
    assert_eq!(mapped, expected);
    let mut folded = Vec::new();
    scan.clone().fold_chunks(
        |_| Vec::new(),
        |part, entry| {
            part.push(entry.record_number);
            Ok(())
        },
        |part| {
            folded.extend(part);
            Ok(())
        },
    )?;
    assert_eq!(folded, expected);
    let mut cached = Vec::new();
    scan.fold_records_cached_enrich(
        Vec::new,
        |part, entry| {
            part.push(entry.record_number);
            Ok(())
        },
        |part| {
            cached.extend(part);
            Ok(())
        },
    )?;
    cached.sort_unstable();
    let mut expected_sorted = expected;
    expected_sorted.sort_unstable();
    assert_eq!(cached, expected_sorted);
    Ok(())
}

#[test]
fn explicit_chunks_filter_unused_and_clip_default_range_and_volume_end() -> MftResult<()> {
    let volume = volume()?;
    let raw = mft(&volume);
    let chunk = RawMftWorkChunk {
        start_record: 0,
        end_record: u64::MAX,
    };
    assert_eq!(
        numbers(raw.parallel().chunks(vec![chunk]).collect_batches()?),
        vec![26]
    );
    let options = RawMftScanOptions::builder()
        .include_unused_records(true)
        .build();
    let scan = raw.parallel().scan_options(options).chunks(vec![chunk]);
    let batches = scan.collect_batches()?;
    assert_eq!(
        batches[0].chunk,
        RawMftWorkChunk {
            start_record: 24,
            end_record: 32
        }
    );
    assert_eq!(numbers(batches), (24..32).collect::<Vec<_>>());
    assert!(
        raw.parallel()
            .chunks(Vec::new())
            .collect_batches()?
            .is_empty()
    );
    Ok(())
}

#[test]
fn explicit_overlaps_are_rejected_after_clipping_before_visiting() -> MftResult<()> {
    let volume = volume()?;
    let raw = mft(&volume);
    for ranges in [
        [(24, 28), (26, 30)],
        [(24, 30), (25, 28)],
        [(24, 28), (24, 28)],
    ] {
        let chunks = ranges
            .map(|(start_record, end_record)| RawMftWorkChunk {
                start_record,
                end_record,
            })
            .to_vec();
        for workers in [1, 3] {
            let scan = raw
                .parallel()
                .chunks(chunks.clone())
                .workers(NonZeroUsize::new(workers).unwrap());
            let mut visited = false;
            let result = scan.for_each_batch(|_| {
                visited = true;
                Ok(())
            });
            assert!(matches!(result, Err(MftError::InvalidOptions(_))));
            assert!(!visited);
        }
    }
    // Overlaps outside the selected range do not invalidate the effective work.
    let chunks = [(0, 10), (5, 15), (24, 28)]
        .map(|(start_record, end_record)| RawMftWorkChunk {
            start_record,
            end_record,
        })
        .to_vec();
    assert_eq!(
        numbers(raw.parallel().chunks(chunks).collect_batches()?),
        vec![26]
    );
    Ok(())
}
