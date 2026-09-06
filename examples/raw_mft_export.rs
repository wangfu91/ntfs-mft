//! Export an extracted MFT or raw volume as JSONL. Diagnostics go to stderr.
use ntfs_mft::export::{ExportContext, JsonlWriter};
use ntfs_mft::{MftError, MftResult, MftStream, MftStreamOptions, RawMftScanOptions};
use std::{
    env,
    io::{self, BufWriter},
    process::ExitCode,
};
mod raw_volume;

fn main() -> ExitCode {
    match run() {
        Ok(false) => ExitCode::SUCCESS,
        Ok(true) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> MftResult<bool> {
    let args: Vec<_> = env::args_os().skip(1).collect();
    let mut writer = JsonlWriter::new(BufWriter::new(io::stdout().lock()));
    let mut errors = 0u64;
    let mut records = 0u64;
    match args.as_slice() {
        [mode, path, size] if mode == "file" => {
            let size = size
                .to_str()
                .and_then(|s| s.parse::<usize>().ok())
                .ok_or(MftError::InvalidOptions("record size must be an integer"))?;
            let options = MftStreamOptions::new(size)?.resolve_extensions(true);
            let mut stream = MftStream::from_path(path, options)?;
            for result in stream.iter_records() {
                match result {
                    Ok(record) => {
                        writer.write_offline(&record)?;
                        records += 1;
                    }
                    Err(error) => {
                        writer.write_error(&error)?;
                        errors += 1;
                    }
                }
            }
        }
        [mode, source] if mode == "volume" => {
            let source = source
                .to_str()
                .ok_or(MftError::InvalidOptions("volume source must be Unicode"))?;
            let (volume, _) = raw_volume::open_raw_volume(Some(source.to_owned()))?;
            let mft = volume.raw_mft()?;
            let options = RawMftScanOptions::builder()
                .start_record(0)
                .include_unused_records(true)
                .collect_alternate_data_streams(true)
                .collect_data_run_summary(true)
                .collect_dos_file_name_links(true)
                .build();
            let context = ExportContext {
                entry_options: Some(options.entry()),
            };
            for result in mft.try_iter_with_options(options)? {
                match result {
                    Ok(entry) => {
                        writer.write_entry(&entry, &context)?;
                        records += 1;
                    }
                    Err(error) => {
                        writer.write_error(&error)?;
                        errors += 1;
                    }
                }
            }
        }
        _ => {
            return Err(MftError::InvalidOptions(
                "usage: raw_mft_export file <MFT-path> <record-size> | volume <drive/mount/device>",
            ));
        }
    }
    writer.flush()?;
    eprintln!("Exported {records} records and {errors} errors.");
    Ok(errors != 0)
}
