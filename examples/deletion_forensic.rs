//! Stream unused raw `$MFT` base records with their names and parent references.
//!
//! Includes bitmap-unused slots and prints records whose FILE header marks them
//! unused. Stops at the result limit; it neither indexes paths nor retains all
//! deleted records. Unused metadata may be stale and is not proof of recovery.
//!
//! On Windows, pass a drive letter from an elevated shell. On Linux and macOS, pass an
//! NTFS mount or device path with read permission for its backing device:
//!
//! ```text
//! cargo run --example deletion_forensic -- /media/user/windows 1000
//! ```

use std::env;

#[path = "../support/raw_volume.rs"]
mod raw_volume;

use ntfs_mft::{
    errors::MftError,
    raw_mft::{RawMft, RawMftScanOptions},
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
        .collect_dos_file_name_links(false)
        .build();

    println!("unused raw MFT records on {source}: showing up to {limit}");
    let mut scan = mft.scan_entries_with_options(options)?;
    let mut count = 0;
    while count < limit {
        let Some(result) = scan.next() else {
            break;
        };
        let record = result?;
        if record.is_used {
            continue;
        }
        let kind = if record.is_directory { "DIR " } else { "FILE" };
        println!(
            "{kind} #{:>10} seq={:>5} parent={} name={}",
            record.record_number,
            record.sequence_number,
            record.parent_reference,
            record.file_name.to_string_lossy(),
        );
        count += 1;
    }
    eprintln!("Done. Yielded {count} unused records (limit {limit}).");
    Ok(())
}
