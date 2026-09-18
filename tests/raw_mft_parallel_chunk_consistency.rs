#![cfg(windows)]

//! Integration test: parallel raw-MFT chunk parsing matches the serial chunk path.
//!
//! The test intentionally samples an early, relatively stable prefix of the MFT
//! so live-filesystem churn is less likely to invalidate the comparison.

use std::num::{NonZeroU64, NonZeroUsize};

use ntfs_mft::{
    NtfsFileReference,
    raw_mft::{RawMft, RawMftBatchEntry, RawMftChunkPlanOptions, RawMftScanOptions},
};

#[path = "../support/live_volume.rs"]
mod live_volume;

#[test]
#[ignore = "requires NTFS_MFT_TEST_DRIVE and Administrator privileges"]
fn raw_mft_parallel_chunk_path_matches_serial_chunk_path() -> Result<(), String> {
    let volume = live_volume::open().map_err(|e| e.to_string())?;
    let raw_mft = RawMft::new(&volume).map_err(|e| e.to_string())?;

    let Some(max_records_per_chunk) = NonZeroU64::new(1024) else {
        return Err("max_records_per_chunk must be non-zero".into());
    };
    let options = RawMftScanOptions::builder()
        .start_record(24)
        .end_record(Some(50_000))
        .collect_alternate_data_streams(false)
        .build();
    let chunk_options = RawMftChunkPlanOptions::builder()
        .max_records_per_chunk(max_records_per_chunk)
        .build();
    let chunks: Vec<_> = raw_mft
        .plan_chunks_with_options(chunk_options, &options)
        .into_iter()
        .take(8)
        .collect();
    if chunks.is_empty() {
        return Err("no work chunks in sampled range; select a populated NTFS volume".into());
    }

    let serial_entries: Vec<_> = chunks
        .iter()
        .map(|chunk| {
            raw_mft
                .read_chunk(*chunk, options.clone())
                .map_err(|error| format!("serial read_chunk failed for {:?}: {error}", chunk))
        })
        .collect::<Result<_, _>>()?;

    let Some(worker_count) = NonZeroUsize::new(4) else {
        return Err("worker_count must be non-zero".into());
    };
    let parallel_batches = raw_mft
        .parallel()
        .lean()
        .chunks(chunks.clone())
        .scan_options(options.into_lean())
        .workers(worker_count)
        .collect_batches()
        .map_err(|error| format!("parallel collect_batches failed: {error}"))?;

    assert_eq!(
        parallel_batches.len(),
        chunks.len(),
        "parallel batch count must match planned chunk count"
    );

    for ((chunk, serial_entries), parallel_batch) in chunks
        .iter()
        .zip(serial_entries.iter())
        .zip(parallel_batches.iter())
    {
        assert_eq!(
            *chunk, parallel_batch.chunk,
            "parallel output must preserve chunk order"
        );
        let serial_identity: Vec<_> = serial_entries.iter().map(entry_identity).collect();
        let parallel_identity: Vec<_> = parallel_batch.entries.iter().map(entry_identity).collect();
        assert_eq!(
            serial_identity, parallel_identity,
            "parallel chunk identity/order must match serial chunk output for {:?}",
            chunk
        );
    }

    Ok(())
}

fn entry_identity(
    entry: &RawMftBatchEntry,
) -> (u64, NtfsFileReference, u64, NtfsFileReference, bool) {
    (
        entry.record_number,
        entry.file_reference,
        entry.base_record_number,
        entry.parent_reference,
        entry.is_directory,
    )
}
