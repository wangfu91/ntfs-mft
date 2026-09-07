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
    volume_with(|_| {})
}

fn volume_with(edit: impl FnOnce(&mut [u8])) -> MftResult<Volume> {
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
    edit(&mut bytes);
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
                    .lean()
                    .workers(NonZeroUsize::new(workers).unwrap())
                    .scheduling(scheduling);
                let first = scan
                    .clone()
                    .scan_options(options.clone().into_lean())
                    .chunk_plan(plan());
                let second = scan
                    .chunk_plan(plan())
                    .scan_options(options.clone().into_lean());
                assert_eq!(
                    first.resolved_chunks()?,
                    raw.plan_chunks_with_options(plan(), &options)
                );
                assert_eq!(numbers(first.collect_batches()?), expected);
                assert_eq!(numbers(second.collect_batches()?), expected);
            }
        }
    }
    assert_eq!(numbers(raw.parallel().lean().collect_batches()?), vec![26]);
    Ok(())
}

#[test]
fn explicit_chunks_intersect_selection_preserving_order() -> Result<(), Box<dyn std::error::Error>>
{
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
        .lean()
        .chunks(chunks.clone())
        .scan_options(options.clone().into_lean())
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
                .lean()
                .scan_options(options.into_lean())
                .chunks(chunks)
                .collect_batches()?
        ),
        expected
    );

    let mut visited = Vec::new();
    scan.clone().try_for_each_batch(|batch| {
        visited.extend(batch.entries.into_iter().map(|e| e.record_number));
        Ok::<_, std::convert::Infallible>(())
    })?;
    assert_eq!(visited, expected);
    let mut mapped = Vec::new();
    scan.clone().try_map_chunks_for_each(
        |batch| Ok::<_, std::convert::Infallible>(numbers(vec![batch])),
        |part| {
            mapped.extend(part);
            Ok::<_, std::convert::Infallible>(())
        },
    )?;
    assert_eq!(mapped, expected);
    let mut folded = Vec::new();
    scan.clone().try_fold_chunks(
        |_| Vec::new(),
        |part, entry| {
            part.push(entry.record_number);
            Ok::<_, std::convert::Infallible>(())
        },
        |part| {
            folded.extend(part);
            Ok::<_, std::convert::Infallible>(())
        },
    )?;
    assert_eq!(folded, expected);
    let mut cached = Vec::new();
    scan.try_fold_records_cached_enrich(
        Vec::new,
        |part, entry| {
            part.push(entry.record_number);
            Ok::<_, std::convert::Infallible>(())
        },
        |part| {
            cached.extend(part);
            Ok::<_, std::convert::Infallible>(())
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
        numbers(
            raw.parallel()
                .lean()
                .chunks(vec![chunk])
                .collect_batches()?
        ),
        vec![26]
    );
    let options = RawMftScanOptions::builder()
        .include_unused_records(true)
        .build();
    let scan = raw
        .parallel()
        .lean()
        .scan_options(options.into_lean())
        .chunks(vec![chunk]);
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
            .lean()
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
                .lean()
                .chunks(chunks.clone())
                .workers(NonZeroUsize::new(workers).unwrap());
            let mut visited = false;
            let result = scan.try_for_each_batch(|_| {
                visited = true;
                Ok::<_, std::convert::Infallible>(())
            });
            assert!(matches!(
                result,
                Err(crate::ScanError::Scanner(MftError::InvalidOptions(_)))
            ));
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
        numbers(raw.parallel().lean().chunks(chunks).collect_batches()?),
        vec![26]
    );
    Ok(())
}

#[test]
fn lean_and_rich_preserve_usage_for_deleted_records() -> MftResult<()> {
    let volume = volume()?;
    let raw = mft(&volume);
    let lean = raw
        .parallel()
        .lean()
        .scan_options(
            RawMftLeanScanOptions::builder()
                .start_record(0)
                .end_record(Some(4))
                .include_unused_records(true)
                .build(),
        )
        .collect_batches()?;
    assert_eq!(
        lean.into_iter()
            .flat_map(|b| b.entries)
            .map(|e| e.is_used)
            .collect::<Vec<_>>(),
        vec![false, false, true, false]
    );
    let rich = raw
        .parallel()
        .rich()
        .scan_options(
            RawMftScanOptions::builder()
                .start_record(0)
                .end_record(Some(4))
                .include_unused_records(true)
                .build(),
        )
        .collect_batches()?;
    assert_eq!(
        rich.into_iter()
            .flat_map(|b| b.entries)
            .map(|e| e.is_used)
            .collect::<Vec<_>>(),
        vec![false, false, true, false]
    );
    Ok(())
}

#[test]
fn path_resolver_construction_propagates_scan_io_errors() -> MftResult<()> {
    let volume = volume()?;
    let mut raw = mft(&volume);
    raw.extent_map = Arc::new(ExtentMap::from_runs(
        &[DataRun::Data {
            lcn: 0,
            clusters: 64,
        }],
        1024,
        1024,
    ));
    raw.bitmap = Arc::from([]);
    assert!(matches!(raw.path_resolver(), Err(MftError::Io(_))));
    Ok(())
}

#[test]
fn diagnostic_scan_counts_skips_bounds_errors_and_retains_metadata_evidence() -> MftResult<()> {
    let volume = volume_with(|bytes| {
        bytes[1024..1028].fill(0); // non-FILE
        bytes[2 * 1024 + 510] = 0; // torn sector
        bytes[3 * 1024 + 510] = 0; // second torn sector
        bytes[4 * 1024 + 32..4 * 1024 + 40].copy_from_slice(&1u64.to_le_bytes()); // extension
    })?;
    let raw = mft(&volume);
    let options = RawMftScanOptions::builder()
        .start_record(0)
        .end_record(Some(6))
        .include_unused_records(true)
        .collect_alternate_data_streams(false)
        .build();
    let mut iter = raw.try_iter_diagnostic(options, 1)?;
    let first = iter.next().unwrap()?;
    assert!(
        !first
            .entry
            .collection_options
            .collect_alternate_data_streams()
    );
    assert_eq!(
        first.metadata.standard_information,
        crate::MftFieldState::Absent
    );
    assert_eq!(
        first.metadata.alternate_data_streams,
        crate::MftFieldState::NotCollected
    );
    assert!(!iter.report().reached_end);
    assert_eq!(
        iter.by_ref()
            .map(|r| r.map(|r| r.entry.record_number))
            .collect::<MftResult<Vec<_>>>()?,
        vec![5]
    );
    assert!(iter.next().is_none());
    let report = iter.into_report();
    assert_eq!(
        (
            report.visited,
            report.yielded,
            report.skipped_non_file,
            report.skipped_extensions,
            report.invalid_records
        ),
        (6, 2, 1, 1, 2)
    );
    assert_eq!(report.diagnostics.len(), 1);
    assert_eq!(report.diagnostics[0].record_number, 2);
    assert!(matches!(
        report.diagnostics[0].error,
        MftError::FixupMismatch { .. }
    ));
    assert_eq!(report.omitted_diagnostics, 1);
    assert!(report.reached_end);
    #[cfg(feature = "serde")]
    assert_eq!(
        crate::export::ExportRecordV1::from_diagnostic(&first)
            .metadata
            .alternate_data_streams,
        crate::export::FieldState::NotCollected
    );
    Ok(())
}

#[test]
fn diagnostic_scan_marks_attribute_lists_unverified_and_stops_on_io() -> MftResult<()> {
    let volume = volume_with(|bytes| {
        // Empty resident ATTRIBUTE_LIST in slot zero.
        bytes[56..60].copy_from_slice(&0x20u32.to_le_bytes());
        bytes[60..64].copy_from_slice(&24u32.to_le_bytes());
        bytes[76..78].copy_from_slice(&24u16.to_le_bytes());
        bytes[80..84].copy_from_slice(&u32::MAX.to_le_bytes());
        bytes[24..28].copy_from_slice(&84u32.to_le_bytes());
    })?;
    let mut raw = mft(&volume);
    raw.extent_map = Arc::new(ExtentMap::from_runs(
        &[DataRun::Data {
            lcn: 0,
            clusters: 64,
        }],
        1024,
        1024,
    ));
    let options = RawMftScanOptions::builder()
        .start_record(0)
        .include_unused_records(true)
        .build();
    let mut iter = raw.try_iter_diagnostic(options, 0)?;
    let first = iter.next().unwrap()?;
    assert_eq!(
        first.metadata.standard_information,
        crate::MftFieldState::Unknown
    );
    assert!(
        first
            .diagnostics
            .iter()
            .any(|d| matches!(d.issue, crate::MftIssue::EnrichmentNotVerified))
    );
    for _ in 1..32 {
        assert!(iter.next().unwrap().is_ok());
    }
    assert!(matches!(iter.next(), Some(Err(MftError::Io(_)))));
    assert!(iter.next().is_none());
    assert!(!iter.report().reached_end);
    Ok(())
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
#[error("database rejected row {row}")]
struct ApplicationError {
    row: u64,
}

#[test]
fn callback_errors_keep_their_type_on_workers_and_the_calling_thread() -> MftResult<()> {
    use crate::ScanError;
    let volume = volume()?;
    let raw = mft(&volume);
    for workers in [1, 3] {
        for scheduling in [
            RawMftParallelScheduling::Dynamic,
            RawMftParallelScheduling::Contiguous,
        ] {
            let config = raw
                .parallel()
                .chunk_plan(plan())
                .workers(NonZeroUsize::new(workers).unwrap())
                .scheduling(scheduling);
            let rich = config.clone().rich().scan_options(
                RawMftScanOptions::builder()
                    .start_record(0)
                    .include_unused_records(true)
                    .build(),
            );
            let lean = config.lean().scan_options(
                RawMftLeanScanOptions::builder()
                    .start_record(0)
                    .include_unused_records(true)
                    .build(),
            );
            let mut visits = 0;
            let result = rich.clone().try_for_each_batch(|_| {
                visits += 1;
                Err(ApplicationError { row: 42 })
            });
            assert!(matches!(
                result,
                Err(ScanError::Consumer(ApplicationError { row: 42 }))
            ));
            assert_eq!(visits, 1);
            let result = lean.clone().try_map_chunks_for_each(
                |_| Err::<(), _>(ApplicationError { row: 43 }),
                |_| Ok(()),
            );
            assert!(matches!(
                result,
                Err(ScanError::Consumer(ApplicationError { row: 43 }))
            ));
            let result =
                rich.try_fold_chunks(|_| (), |_, _| Err(ApplicationError { row: 44 }), |_| Ok(()));
            assert!(matches!(
                result,
                Err(ScanError::Consumer(ApplicationError { row: 44 }))
            ));
            let result = lean.clone().try_fold_chunks(
                |_| (),
                |_, _| Err(ApplicationError { row: 45 }),
                |_| Ok(()),
            );
            assert!(matches!(
                result,
                Err(ScanError::Consumer(ApplicationError { row: 45 }))
            ));
            let result = lean.try_fold_records_cached_enrich(
                || (),
                |_, _| Err(ApplicationError { row: 46 }),
                |_| Ok(()),
            );
            assert!(matches!(
                result,
                Err(ScanError::Consumer(ApplicationError { row: 46 }))
            ));
        }
    }
    Ok(())
}

#[test]
fn typed_callbacks_distinguish_scanner_errors_and_preserve_success_order() -> MftResult<()> {
    use crate::ScanError;
    let volume = volume()?;
    let mut raw = mft(&volume);
    let mut records = Vec::new();
    let scan = raw.parallel().lean().chunk_plan(plan()).scan_options(
        RawMftLeanScanOptions::builder()
            .start_record(0)
            .include_unused_records(true)
            .build(),
    );
    let result = scan.try_for_each_batch(|batch| -> Result<(), ApplicationError> {
        records.extend(batch.entries.into_iter().map(|e| e.record_number));
        Ok(())
    });
    assert!(result.is_ok());
    assert_eq!(records, (0..32).collect::<Vec<_>>());
    raw.extent_map = Arc::new(ExtentMap::from_runs(
        &[DataRun::Data {
            lcn: 0,
            clusters: 64,
        }],
        1024,
        1024,
    ));
    let result = raw
        .parallel()
        .rich()
        .scan_options(
            RawMftScanOptions::builder()
                .include_unused_records(true)
                .build(),
        )
        .try_for_each_batch(|_| Ok::<_, ApplicationError>(()));
    assert!(matches!(result, Err(ScanError::Scanner(MftError::Io(_)))));
    Ok(())
}
