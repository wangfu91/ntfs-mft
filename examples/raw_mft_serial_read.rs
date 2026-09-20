//! Preview up to 1,000 entries in the raw `$MFT` of a drive and print a one-line
//! summary that highlights the rich metadata `RawMft` exposes (full
//! timestamps, real / allocated size, alternate data streams, sparse and
//! compressed flags).
//!
//! On Windows, pass a drive letter from an elevated shell. On Linux and macOS, pass an
//! NTFS mount or device path with read permission for its backing device:
//!
//! ```text
//! cargo run --example raw_mft_serial_read -- /media/user/windows
//! ```

//! Timings exclude compilation and elevation; scan/output includes terminal I/O.

use std::{env, time::Instant};

#[path = "../support/raw_volume.rs"]
mod raw_volume;

use ntfs_mft::{errors::MftError, raw_mft::RawMft};

const MAX_ENTRIES: usize = 1_000;

/// Run the example and print any top-level error.
fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

/// Open the raw `$MFT`, iterate records, and print a compact metadata summary.
fn run() -> Result<(), MftError> {
    let mut args = env::args().skip(1);
    let source = args.next();
    if args.next().is_some() {
        return Err(MftError::InvalidOptions(
            "usage: raw_mft_serial_read [<drive/mount/device>]",
        ));
    }

    let started = Instant::now();
    let (volume, _) = raw_volume::open_raw_volume(source)?;
    eprintln!("Open volume: {:.3?}", started.elapsed());

    let started = Instant::now();
    let mft = RawMft::new(&volume)?;
    eprintln!("MFT bootstrap: {:.3?}", started.elapsed());

    println!(
        "$MFT: {} records, cluster_size={}, file_record_size={}",
        mft.record_count(),
        mft.cluster_size(),
        mft.file_record_size()
    );

    let started = Instant::now();
    let mut count = 0u64;
    for result in mft.scan_entries()?.take(MAX_ENTRIES) {
        let entry = result?;
        let kind = if entry.is_directory { "DIR " } else { "FILE" };
        println!(
            "{kind} #{:>10} size={:>12} alloc={:>12} links={} ads={} sparse={} compressed={} parent={} name={}",
            entry.record_number,
            entry.real_size,
            entry.allocated_size,
            entry.hard_link_count,
            entry.alternate_data_streams.len(),
            entry.is_sparse,
            entry.is_compressed,
            entry.parent_reference,
            entry.file_name.to_string_lossy(),
        );
        count += 1;
    }
    eprintln!("Scan + output: {:.3?}", started.elapsed());
    eprintln!("Done. Yielded {count} records.");
    Ok(())
}
