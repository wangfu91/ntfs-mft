//! Exact-match profiling target for the serial raw-`$MFT` read benchmark.
//!
//! Drives the same [`run_serial_read`] workload as `benches/raw_mft.rs` so
//! flamegraph and ETW captures reflect the benchmarked code path.
//!
//! ```text
//! cargo flamegraph -o raw_mft_serial_read.svg --example raw_mft_serial_read_profile
//! ```

use std::{error::Error, time::Instant};

use ntfs_mft::raw_mft::RawMft;

#[path = "../../support/raw_mft_serial_support.rs"]
mod serial_support;

use serial_support::{SerialConfig, open_volume, print_serial_config, run_serial_read};

fn main() -> Result<(), Box<dyn Error>> {
    let config = SerialConfig::from_env();
    print_serial_config(&config);

    let volume = open_volume(config.drive)?;
    let mft = RawMft::new(&volume)?;

    // Warm the cache so the timed pass measures parser CPU, matching the bench.
    let _ = run_serial_read(&mft, &config)?;

    let start = Instant::now();
    let summary = run_serial_read(&mft, &config)?;
    let elapsed = start.elapsed();

    println!("raw_mft serial read profile");

    println!("  records:         {}", summary.records);
    println!("  errors:          {}", summary.errors);
    println!("  name_chars:      {}", summary.name_chars);
    println!("  links:           {}", summary.links);
    println!("  ads:             {}", summary.ads);
    println!("  elapsed:         {:.3}s", elapsed.as_secs_f64());

    Ok(())
}
