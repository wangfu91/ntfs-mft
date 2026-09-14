//! Scan a raw NTFS `$MFT` in parallel and report chunk throughput.
//!
//! Usage: `raw_mft_parallel_chunks <drive/mount/device> [records-per-chunk] [workers]`.

use std::{
    env,
    error::Error,
    num::{NonZeroU64, NonZeroUsize},
    thread,
    time::Instant,
};

use ntfs_mft::raw_mft::{RawMft, RawMftChunkPlanOptions, RawMftScanOptions};

#[path = "../support/raw_volume.rs"]
mod raw_volume;

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = env::args().skip(1);
    let source_arg = args.next();
    let max_records_per_chunk = args
        .next()
        .map(|arg| {
            arg.parse::<u64>()
                .ok()
                .and_then(NonZeroU64::new)
                .ok_or("records per chunk must be a positive integer")
        })
        .transpose()?
        .unwrap_or(NonZeroU64::new(16 * 1024).unwrap_or(NonZeroU64::MIN));
    let worker_count = args
        .next()
        .map(|arg| {
            arg.parse::<usize>()
                .ok()
                .and_then(NonZeroUsize::new)
                .ok_or("worker count must be a positive integer")
        })
        .transpose()?
        .unwrap_or(
            thread::available_parallelism()
                .ok()
                .unwrap_or(NonZeroUsize::MIN),
        );
    if args.next().is_some() {
        return Err(
            "usage: raw_mft_parallel_chunks <drive/mount/device> [records-per-chunk] [workers]"
                .into(),
        );
    }

    let (volume, source) = raw_volume::open_raw_volume(source_arg)?;
    let raw_mft = RawMft::new(&volume)?;
    let chunk_plan = RawMftChunkPlanOptions::builder()
        .max_records_per_chunk(max_records_per_chunk)
        .build();
    let options = RawMftScanOptions::builder()
        .include_unused_records(false)
        .collect_alternate_data_streams(false)
        .collect_dos_file_name_links(false)
        .build();

    let chunks = raw_mft.plan_chunks_with_options(chunk_plan, &options);

    let start = Instant::now();
    let chunk_count = chunks.len();
    let mut entry_count = 0usize;
    raw_mft
        .parallel()
        .lean()
        .chunks(chunks)
        .scan_options(options.into_lean())
        .workers(worker_count)
        .try_for_each_batch(|batch| {
            entry_count += batch.entries.len();
            Ok::<_, std::convert::Infallible>(())
        })?;
    let elapsed = start.elapsed();

    println!("raw_mft parallel chunks");
    println!("  volume:                   {source}");
    println!("  worker_count:             {}", worker_count);
    println!("  max_records_per_chunk:    {}", max_records_per_chunk);
    println!("  chunk_count:              {chunk_count}");
    println!("  entry_count:              {entry_count}");
    println!("  elapsed:                  {:.3}s", elapsed.as_secs_f64());
    if elapsed.as_secs_f64() > 0.0 {
        println!(
            "  entries_per_second:       {:.0}",
            entry_count as f64 / elapsed.as_secs_f64()
        );
    }

    Ok(())
}
