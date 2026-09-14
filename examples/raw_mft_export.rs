//! Export an extracted MFT or raw volume as JSONL. Diagnostics go to stderr.
use ntfs_mft::export::JsonlWriter;
use ntfs_mft::{
    MftError, MftIssue, MftResult, MftStream, MftStreamOptions, MftStreamRecord, RawMftScanOptions,
};
use std::{
    env,
    io::{self, BufWriter},
    process::ExitCode,
};

#[path = "../support/raw_volume.rs"]
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
    let usage = || {
        MftError::InvalidOptions(
            "usage: raw_mft_export file <MFT-path> <record-size> | volume <drive/mount/device>",
        )
    };
    let (command, arguments) = args.split_first().ok_or_else(usage)?;

    let mut writer = JsonlWriter::new(BufWriter::new(io::stdout().lock()));
    let mut errors = 0u64;
    let mut records = 0u64;
    match (command.to_str(), arguments) {
        (Some("file"), [path, size]) => {
            let size = size
                .to_str()
                .and_then(|s| s.parse::<usize>().ok())
                .ok_or(MftError::InvalidOptions("record size must be an integer"))?;
            let options = MftStreamOptions::new(size)?.resolve_extensions(true);
            let mut stream = MftStream::from_path(path, options)?;
            (records, errors) = export_offline(stream.iter_records(), &mut writer)?;
        }
        (Some("volume"), [source]) => {
            let source = source
                .to_str()
                .ok_or(MftError::InvalidOptions("volume source must be Unicode"))?;
            let (volume, _) = raw_volume::open_raw_volume(Some(source.to_owned()))?;
            let mft = volume.raw_mft()?;
            let options = RawMftScanOptions::builder()
                .start_record(0)
                .include_unused_records(true)
                .collect_alternate_data_streams(true)
                .collect_dos_file_name_links(true)
                .build();
            for result in mft.try_iter_with_options(options)? {
                match result {
                    Ok(entry) => {
                        writer.write_entry(&entry)?;
                        records += 1;
                    }
                    Err(error) => {
                        writer.write_error(&error)?;
                        errors += 1;
                    }
                }
            }
        }
        _ => return Err(usage()),
    }
    writer.flush()?;
    eprintln!("Exported {records} records; {errors} failures (including terminal diagnostics).");
    Ok(errors != 0)
}

fn export_offline(
    source: impl IntoIterator<Item = MftResult<MftStreamRecord>>,
    writer: &mut JsonlWriter<impl io::Write>,
) -> MftResult<(u64, u64)> {
    let (mut records, mut errors) = (0, 0);
    for result in source {
        match result {
            Ok(record) => {
                let terminal = record.diagnostics.iter().any(|diagnostic| {
                    matches!(
                        &diagnostic.issue,
                        MftIssue::ExtensionRead { error, .. } if error.is_io_error()
                    )
                });
                writer.write_stream(&record)?;
                records += 1;
                if terminal {
                    // The offline iterator stops here even though the item is Ok.
                    // Keep its usable metadata, but never report this as a complete scan.
                    errors += 1;
                    break;
                }
            }
            Err(error) => {
                writer.write_error(&error)?;
                errors += 1;
                if error.is_io_error() {
                    break;
                }
            }
        }
    }
    Ok((records, errors))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Read, Seek, SeekFrom};

    struct FailsOnExtension(Cursor<Vec<u8>>);
    impl Read for FailsOnExtension {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            if self.0.position() >= 1024 {
                return Err(io::Error::other("extension device read failed"));
            }
            self.0.read(buffer)
        }
    }
    impl Seek for FailsOnExtension {
        fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
            self.0.seek(position)
        }
    }

    #[test]
    fn interrupted_extension_export_preserves_record_and_reports_failure()
    -> Result<(), Box<dyn std::error::Error>> {
        // Slot zero: USA-protected FILE with a resident ATTRIBUTE_LIST referencing
        // DATA instance 2 in slot one, sequence 7. Reading slot one fails with I/O.
        let mut bytes = vec![0u8; 2048];
        bytes[..4].copy_from_slice(b"FILE");
        bytes[4..6].copy_from_slice(&48u16.to_le_bytes());
        bytes[6..8].copy_from_slice(&3u16.to_le_bytes());
        bytes[16..18].copy_from_slice(&7u16.to_le_bytes());
        bytes[20..22].copy_from_slice(&56u16.to_le_bytes());
        bytes[22..24].copy_from_slice(&1u16.to_le_bytes());
        bytes[24..28].copy_from_slice(&116u32.to_le_bytes());
        bytes[28..32].copy_from_slice(&1024u32.to_le_bytes());
        bytes[48..50].copy_from_slice(&0xabcdu16.to_le_bytes());
        for offset in [510, 1022] {
            bytes[offset..offset + 2].copy_from_slice(&0xabcdu16.to_le_bytes());
        }
        bytes[56..60].copy_from_slice(&0x20u32.to_le_bytes());
        bytes[60..64].copy_from_slice(&56u32.to_le_bytes());
        bytes[72..76].copy_from_slice(&32u32.to_le_bytes());
        bytes[76..78].copy_from_slice(&24u16.to_le_bytes());
        bytes[80..84].copy_from_slice(&0x80u32.to_le_bytes());
        bytes[84..86].copy_from_slice(&32u16.to_le_bytes());
        bytes[96..104].copy_from_slice(&((7u64 << 48) | 1).to_le_bytes());
        bytes[104..106].copy_from_slice(&2u16.to_le_bytes());
        bytes[112..116].copy_from_slice(&u32::MAX.to_le_bytes());
        let mut source = MftStream::from_reader(
            FailsOnExtension(Cursor::new(bytes)),
            MftStreamOptions::new(1024)?.resolve_extensions(true),
        )?;
        let mut writer = JsonlWriter::new(Vec::new());
        let (records, failures) = export_offline(source.iter_records(), &mut writer)?;
        assert_eq!((records, failures), (1, 1));
        let output = writer.finish()?;
        let item: serde_json::Value = serde_json::from_slice(&output)?;
        assert_eq!(item["type"], "record");
        assert_eq!(item["data"]["diagnostics"][0]["code"], "extension_read");
        assert_eq!(item["data"]["diagnostics"][0]["error"]["category"], "io");
        Ok(())
    }
}
