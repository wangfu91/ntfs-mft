//! Enumerate deleted / unused raw `$MFT` records and print a best-effort path.
//!
//! This example scans the raw `$MFT` with `include_unused_records(true)` so it
//! can see slots whose `$BITMAP` bit is clear, then builds a snapshot-local
//! historical path index from all named base records.
//!
//! Resolution quality tags:
//! - `exact`: every component matched the exact `NtfsFileReference` stored in the record.
//! - `record-fallback`: at least one parent step had to ignore stale sequence
//!   bits and use the current snapshot occupant of that record number.
//! - `partial`: the chain broke before reaching the root, so the printed path
//!   includes an unresolved-parent marker.
//!
//! Usage: `deletion_forensic <drive/mount/device> [result-limit]`.

use std::env;

#[path = "../support/raw_volume.rs"]
mod raw_volume;

use ntfs_mft::{
    errors::MftError,
    raw_mft::{RawMft, RawMftScanOptions, history::HistoricalPathIndex},
};

const DEFAULT_LIMIT: usize = 1_000;

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), MftError> {
    let mut args = env::args().skip(1);
    let (volume, source) = raw_volume::open_raw_volume(args.next())?;
    let limit = args
        .next()
        .map(|value| {
            value
                .parse::<usize>()
                .map_err(|_| MftError::InvalidOptions("result limit must be an integer"))
        })
        .transpose()?
        .unwrap_or(DEFAULT_LIMIT);
    if args.next().is_some() {
        return Err(MftError::InvalidOptions(
            "usage: deletion_forensic <drive/mount/device> [result-limit]",
        ));
    }

    let mft = RawMft::new(&volume)?;
    let options = RawMftScanOptions::builder()
        .include_unused_records(true)
        .collect_alternate_data_streams(false)
        .collect_data_run_summary(false)
        .collect_dos_file_name_links(false)
        .build();

    let mut index = HistoricalPathIndex::new();
    let mut deleted_records = Vec::new();

    for result in mft.try_iter_with_options(options)? {
        let entry = match result {
            Ok(entry) => entry,
            Err(error) => {
                eprintln!("error: {error}");
                continue;
            }
        };

        index.insert(&entry);
        if !entry.is_used {
            deleted_records.push(entry);
        }
    }

    println!(
        "unused raw MFT records on {source}: ({} total, showing up to {})",
        deleted_records.len(),
        limit
    );

    for record in deleted_records.iter().take(limit) {
        let resolved = index.resolve_entry_best_effort(&volume, record);
        let kind = if record.is_directory { "DIR " } else { "FILE" };
        println!(
            "{kind} #{:>10} seq={:>5} quality={:<15} {}",
            record.record_number,
            record.sequence_number,
            resolved.quality,
            resolved.path.display(),
        );
    }

    if deleted_records.len() > limit {
        eprintln!("truncated output at {limit} records; rerun with a higher limit to inspect more");
    }

    Ok(())
}
