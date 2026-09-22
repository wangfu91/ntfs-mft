//! Criterion benchmark for the serial raw-`$MFT` read path.
//!
//! Run on an elevated shell with:
//!
//! ```text
//! cargo bench --bench raw_mft -- --sample-size 20
//! ```
//!
//! Save a baseline before an optimization wave with:
//!
//! ```text
//! cargo bench --bench raw_mft -- --sample-size 20 --save-baseline serial-start
//! ```
//!
//! Compare a change against that baseline with:
//!
//! ```text
//! cargo bench --bench raw_mft -- --sample-size 20 --baseline serial-start
//! ```
//!
//! Profile the exact same workload with:
//!
//! ```text
//! cargo flamegraph -o raw_mft_serial_read.svg --example raw_mft_serial_read_profile
//! ```
//!
//! On Windows, set `NTFS_MFT_TEST_DRIVE` to choose the drive letter (default `C`).
//! On Linux and macOS, set `NTFS_MFT_TEST_VOLUME` to an NTFS mount or device path. All benches
//! skip gracefully when the source isn't NTFS or cannot be read.
//! Set `NTFS_MFT_SERIAL_MAX_RECORDS` to cap the per-run record count for
//! faster optimization-loop iterations.

use std::time::Duration;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use ntfs_mft::raw_mft::RawMft;

#[path = "../support/raw_mft_serial_support.rs"]
mod serial_support;

use serial_support::{
    SerialConfig, SerialSummary, nonzero_usize, open_volume, print_serial_config, run_serial_read,
};

/// Main-buffer sizes (in bytes) swept by the optional buffer-size group.
const BUFFER_SWEEP_BYTES: [usize; 4] = [64 * 1024, 256 * 1024, 1024 * 1024, 4 * 1024 * 1024];

fn raw_mft_serial_benchmarks(c: &mut Criterion) {
    let config = SerialConfig::from_env();
    print_serial_config(&config);

    let volume = match open_volume(config.drive) {
        Ok(volume) => volume,
        Err(error) => {
            eprintln!("skipping bench: {error}");
            return;
        }
    };
    let mft = match RawMft::new(&volume) {
        Ok(mft) => mft,
        Err(error) => {
            eprintln!("skipping bench: {error}");
            return;
        }
    };

    // Warm the OS cache and report the workload shape so the measured run
    // reflects parser CPU rather than first-touch cold reads.
    match run_serial_read(&mft, &config) {
        Ok(summary) => print_summary(&summary),
        Err(error) => {
            eprintln!("skipping bench: {error}");
            return;
        }
    }

    let mut group = c.benchmark_group("raw_mft_serial");
    group.bench_function("read", |b| {
        b.iter(|| {
            let summary = run_serial_read(&mft, &config).unwrap_or_else(|error| fail(error));
            std::hint::black_box(summary)
        });
    });
    group.finish();

    if buffer_sweep_enabled() {
        let mut sweep = c.benchmark_group("raw_mft_serial_buffer");
        for bytes in BUFFER_SWEEP_BYTES {
            let buffer_bytes = nonzero_usize(bytes);
            let bench_id = BenchmarkId::new("read", bytes);
            sweep.bench_with_input(bench_id, &buffer_bytes, |b, &buffer_bytes| {
                b.iter(|| {
                    let sweep_config = config.with_buffer_bytes(buffer_bytes);
                    let summary =
                        run_serial_read(&mft, &sweep_config).unwrap_or_else(|error| fail(error));
                    std::hint::black_box(summary)
                });
            });
        }
        sweep.finish();
    }
}

fn fail(error: impl std::fmt::Display) -> ! {
    eprintln!("serial read benchmark failed: {error}");
    std::process::exit(1)
}

/// Whether the optional buffer-size sweep group should run.
fn buffer_sweep_enabled() -> bool {
    std::env::var_os("NTFS_MFT_SERIAL_BUFFER_SWEEP").is_some()
}

/// Print the warm-up workload summary so each run records its shape.
fn print_summary(summary: &SerialSummary) {
    eprintln!(
        "raw_mft serial workload: records={} errors={} name_chars={} links={} ads={}",
        summary.records, summary.errors, summary.name_chars, summary.links, summary.ads,
    );
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .sample_size(20)
        .warm_up_time(Duration::from_secs(3))
        .measurement_time(Duration::from_secs(30))
        .configure_from_args();
    targets = raw_mft_serial_benchmarks
}
criterion_main!(benches);
