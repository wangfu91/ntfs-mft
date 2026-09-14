//! Profiling target for the parallel ingest workload in `benches/raw_mft_ingest.rs`.
//!
//! Configure it with the `NTFS_MFT_TEST_*` and `NTFS_MFT_BENCH_*` environment
//! variables documented in the repository's performance notes.

use std::{error::Error, time::Instant};

use ntfs_mft::raw_mft::RawMft;

#[path = "../../support/raw_mft_ingest_support.rs"]
mod ingest_support;

use ingest_support::{bench_config, open_volume, print_bench_config, run_parallel_ingest};

fn main() -> Result<(), Box<dyn Error>> {
    let config = bench_config().clone();
    print_bench_config(&config);

    let volume = open_volume(config.drive)?;
    let raw_mft = RawMft::new(&volume)?;

    let start = Instant::now();
    let summary = run_parallel_ingest(&raw_mft, &config)?;
    let elapsed = start.elapsed();

    println!("raw_mft parallel ingest profile");

    println!("  records:                  {}", summary.records);
    println!("  parent_buckets:           {}", summary.parent_buckets);
    println!("  child_links:              {}", summary.child_links);
    println!("  logical_bytes:            {}", summary.logical_bytes);
    println!("  allocated_bytes:          {}", summary.allocated_bytes);
    println!("  elapsed:                  {:.3}s", elapsed.as_secs_f64());

    Ok(())
}
