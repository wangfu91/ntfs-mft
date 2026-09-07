#![cfg(target_os = "macos")]

//! Opt-in macOS device validation. An explicitly supplied source must succeed.

use ntfs_mft::{
    MftResult, MftStream, MftStreamOptions, RawMftChunkPlanOptions, RawMftParallelScheduling,
    RawMftScanOptions, Volume,
};
use std::{
    io::{Read, Seek, SeekFrom},
    num::{NonZeroU64, NonZeroUsize},
    path::Path,
};

#[test]
fn macos_volume_serial_parallel_and_owned_byte_stream() -> MftResult<()> {
    let Some(source) = std::env::var_os("NTFS_MFT_TEST_VOLUME") else {
        eprintln!("SKIP: set NTFS_MFT_TEST_VOLUME to an NTFS mount root or /dev/diskNsM");
        return Ok(());
    };
    let source = Path::new(&source);
    let volume = if source.starts_with("/dev") {
        let volume = Volume::from_device_path(source)?;
        assert_eq!(volume.device_path(), Some(source));
        volume
    } else {
        let volume = Volume::from_mount_point(source)?;
        assert_eq!(volume.mount_point(), Some(source.canonicalize()?.as_path()));
        volume
    };
    let raw = volume.raw_mft()?;
    let options = RawMftScanOptions::builder().end_record(Some(4096)).build();
    let serial = raw
        .try_iter_with_options(options.clone())?
        .collect::<MftResult<Vec<_>>>()?;
    assert!(!serial.is_empty());
    let chunks = raw.plan_chunks_with_options(
        RawMftChunkPlanOptions::builder()
            .max_records_per_chunk(NonZeroU64::new(16).unwrap())
            .build(),
        &options,
    );
    assert!(
        chunks.len() > 1,
        "test must exercise multiple worker chunks"
    );
    let expected = serial
        .iter()
        .map(|e| {
            (
                e.record_number,
                e.file_reference,
                e.parent_reference,
                e.is_directory,
            )
        })
        .collect::<Vec<_>>();
    for scheduling in [
        RawMftParallelScheduling::Dynamic,
        RawMftParallelScheduling::Contiguous,
    ] {
        let batches = raw
            .parallel()
            .lean()
            .chunks(chunks.clone())
            .scan_options(options.clone().into_lean())
            .workers(NonZeroUsize::new(4).unwrap())
            .scheduling(scheduling)
            .collect_batches()?;
        assert_eq!(batches.iter().map(|b| b.chunk).collect::<Vec<_>>(), chunks);
        let actual = batches
            .iter()
            .flat_map(|b| &b.entries)
            .map(|e| {
                (
                    e.record_number,
                    e.file_reference,
                    e.parent_reference,
                    e.is_directory,
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            actual, expected,
            "serial and parallel metadata/order must match"
        );
    }
    let root = raw.read_record(5)?.unwrap();
    let record_size = raw.file_record_size();
    let mut first = raw.byte_reader()?;
    let mut second = raw.byte_reader()?;
    second.seek(SeekFrom::Start(record_size as u64 * 5))?;
    let mut signature = [0; 4];
    second.read_exact(&mut signature)?;
    assert_eq!(&signature, b"FILE");
    first.read_exact(&mut signature)?;
    assert_eq!(&signature, b"FILE");
    drop(second);
    drop(raw);
    drop(volume);
    let mut stream = MftStream::from_reader(first, MftStreamOptions::new(record_size)?)?;
    let parsed = stream.read_record(5)?;
    assert_eq!(parsed.entry.file_reference, root.file_reference);
    assert_eq!(parsed.entry.file_name, root.file_name);
    assert!(parsed.entry.is_directory);
    eprintln!(
        "Validated {} serial records, {} chunks with both parallel schedulers, and an independently owned MFT stream",
        serial.len(),
        chunks.len()
    );
    Ok(())
}
