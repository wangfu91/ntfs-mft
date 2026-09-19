//! Differential coverage for the live entry scan and logical-stream record iterator.

use super::super::{
    RawMft,
    layout::{boot::BootSector, data_run::DataRun, extent::ExtentMap},
};
use crate::{
    MftDiagnosticKind, MftError, MftFieldState, MftResult, MftStream, MftStreamOptions,
    RawMftReadBuffers, RawMftScanOptions, Volume, volume::sparse_fixture::SparseVolumeFixture,
};
use std::{
    io::{self, Cursor, Read, Seek, SeekFrom},
    num::NonZeroUsize,
    sync::Arc,
};

const RECORD_SIZE: usize = 1024;

fn resident(kind: u32, id: u16, name: &str, value: &[u8]) -> Vec<u8> {
    let name: Vec<u8> = name.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let value_offset = (24 + name.len()).div_ceil(8) * 8;
    let len = (value_offset + value.len()).div_ceil(8) * 8;
    let mut attribute = vec![0; len];
    attribute[..4].copy_from_slice(&kind.to_le_bytes());
    attribute[4..8].copy_from_slice(&(len as u32).to_le_bytes());
    attribute[9] = (name.len() / 2) as u8;
    attribute[10..12].copy_from_slice(&24u16.to_le_bytes());
    attribute[14..16].copy_from_slice(&id.to_le_bytes());
    attribute[16..20].copy_from_slice(&(value.len() as u32).to_le_bytes());
    attribute[20..22].copy_from_slice(&(value_offset as u16).to_le_bytes());
    attribute[24..24 + name.len()].copy_from_slice(&name);
    attribute[value_offset..value_offset + value.len()].copy_from_slice(value);
    attribute
}

fn attribute_list_reference(
    kind: u32,
    record_number: u64,
    sequence: u16,
    id: u16,
    name: &str,
    lowest_vcn: i64,
) -> Vec<u8> {
    let name: Vec<u8> = name.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let len = (26 + name.len()).div_ceil(8) * 8;
    let mut reference = vec![0; len];
    reference[..4].copy_from_slice(&kind.to_le_bytes());
    reference[4..6].copy_from_slice(&(len as u16).to_le_bytes());
    reference[6] = (name.len() / 2) as u8;
    reference[7] = 26;
    reference[8..16].copy_from_slice(&lowest_vcn.to_le_bytes());
    reference[16..24].copy_from_slice(&(record_number | ((sequence as u64) << 48)).to_le_bytes());
    reference[24..26].copy_from_slice(&id.to_le_bytes());
    reference[26..26 + name.len()].copy_from_slice(&name);
    reference
}

fn record(base_reference: u64, attributes: &[Vec<u8>]) -> Vec<u8> {
    let usa_count = RECORD_SIZE / 512 + 1;
    let attributes_offset = (48 + usa_count * 2).div_ceil(8) * 8;
    let mut bytes = vec![0; RECORD_SIZE];
    bytes[..4].copy_from_slice(b"FILE");
    bytes[4..6].copy_from_slice(&48u16.to_le_bytes());
    bytes[6..8].copy_from_slice(&(usa_count as u16).to_le_bytes());
    bytes[16..18].copy_from_slice(&7u16.to_le_bytes());
    bytes[18..20].copy_from_slice(&1u16.to_le_bytes());
    bytes[20..22].copy_from_slice(&(attributes_offset as u16).to_le_bytes());
    bytes[22..24].copy_from_slice(&1u16.to_le_bytes());
    bytes[28..32].copy_from_slice(&(RECORD_SIZE as u32).to_le_bytes());
    bytes[32..40].copy_from_slice(&base_reference.to_le_bytes());

    let mut offset = attributes_offset;
    for attribute in attributes {
        bytes[offset..offset + attribute.len()].copy_from_slice(attribute);
        offset += attribute.len();
    }
    bytes[offset..offset + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    bytes[24..28].copy_from_slice(&((offset + 4) as u32).to_le_bytes());

    bytes[48..50].copy_from_slice(&0xabcd_u16.to_le_bytes());
    for sector in 1..usa_count {
        let trailer = sector * 512 - 2;
        let original = [bytes[trailer], bytes[trailer + 1]];
        bytes[48 + sector * 2..50 + sector * 2].copy_from_slice(&original);
        bytes[trailer..trailer + 2].copy_from_slice(&0xabcd_u16.to_le_bytes());
    }
    bytes
}

fn volume_from_records(records: &[Vec<u8>]) -> MftResult<Volume> {
    Arc::new(SparseVolumeFixture::new(vec![(0, records.concat())])).open()
}

fn raw_mft(volume: &Volume, record_count: u64) -> RawMft<'_> {
    RawMft {
        volume,
        boot: BootSector {
            bytes_per_sector: 512,
            mft_lcn: 0,
            file_record_size: RECORD_SIZE as u64,
            cluster_size: RECORD_SIZE as u64,
            mft_byte_offset: 0,
        },
        extent_map: Arc::new(ExtentMap::from_runs(
            &[DataRun::Data {
                lcn: 0,
                clusters: record_count,
            }],
            RECORD_SIZE as u64,
            RECORD_SIZE as u64,
        )),
        bitmap: Arc::from([]),
    }
}

fn scan_options(record_count: u64) -> RawMftScanOptions {
    let record_buffer = NonZeroUsize::new(RECORD_SIZE).unwrap();
    RawMftScanOptions::builder()
        .range(0..record_count)
        .include_unused_records(true)
        .buffers(RawMftReadBuffers::new(record_buffer, record_buffer))
        .build()
}

fn stream_options() -> MftResult<MftStreamOptions> {
    MftStreamOptions::new(RECORD_SIZE)
}

#[test]
fn file_and_usa_outcomes_correspond_by_record_position() -> MftResult<()> {
    let valid_zero = record(0, &[]);
    let non_file = vec![0; RECORD_SIZE];
    let mut torn = record(0, &[]);
    torn[510] ^= 1;
    let mut bad_geometry = record(0, &[]);
    bad_geometry[28..32].copy_from_slice(&512u32.to_le_bytes());
    let valid_four = record(0, &[resident(0x80, 1, "", b"payload")]);
    let records = vec![valid_zero, non_file, torn, bad_geometry, valid_four];

    let volume = volume_from_records(&records)?;
    let raw = raw_mft(&volume, records.len() as u64);
    let scanned = raw
        .scan_entries_with_options(scan_options(records.len() as u64))?
        .collect::<MftResult<Vec<_>>>()?;
    assert_eq!(
        scanned
            .iter()
            .map(|entry| entry.record_number)
            .collect::<Vec<_>>(),
        [0, 4]
    );

    let mut stream = MftStream::from_reader(Cursor::new(records.concat()), stream_options()?)?;
    let mut records = stream.iter_records();
    let stream_zero = records.next().unwrap()?;
    assert!(matches!(
        records.next(),
        Some(Err(MftError::InvalidMftRecord { number: 1, .. }))
    ));
    assert!(matches!(
        records.next(),
        Some(Err(MftError::FixupMismatch { number: 2 }))
    ));
    assert!(matches!(
        records.next(),
        Some(Err(MftError::InvalidMftRecord { number: 3, .. }))
    ));
    let stream_four = records.next().unwrap()?;
    assert!(records.next().is_none());
    assert_eq!(scanned[0], stream_zero.entry);
    assert_eq!(scanned[1], stream_four.entry);
    Ok(())
}

#[test]
fn malformed_attributes_preserve_the_same_entry_projection() -> MftResult<()> {
    let mut malformed = record(0, &[]);
    let attributes_offset = u16::from_le_bytes([malformed[20], malformed[21]]) as usize;
    malformed[attributes_offset..attributes_offset + 4].copy_from_slice(&0x80u32.to_le_bytes());
    let following = record(0, &[resident(0x80, 1, "", b"following")]);
    let records = vec![malformed, following];

    let volume = volume_from_records(&records)?;
    let raw = raw_mft(&volume, records.len() as u64);
    let scanned = raw
        .scan_entries_with_options(scan_options(records.len() as u64))?
        .collect::<MftResult<Vec<_>>>()?;

    let mut stream = MftStream::from_reader(Cursor::new(records.concat()), stream_options()?)?;
    let malformed = stream.read_record(0)?;
    assert_eq!(malformed.metadata.unnamed_data, MftFieldState::Unknown);
    assert!(
        malformed
            .diagnostics
            .iter()
            .any(|diagnostic| matches!(diagnostic.kind, MftDiagnosticKind::MalformedAttribute))
    );
    assert_eq!(scanned[0], malformed.entry);
    assert_eq!(scanned[1], stream.read_record(1)?.entry);
    Ok(())
}

#[test]
fn base_and_extension_enrichment_produce_the_same_base_entry() -> MftResult<()> {
    let list = attribute_list_reference(0x80, 1, 7, 2, "", 0);
    let records = vec![
        record(0, &[resident(0x20, 1, "", &list)]),
        record(7 << 48, &[resident(0x80, 2, "", b"extension payload")]),
    ];

    let volume = volume_from_records(&records)?;
    let raw = raw_mft(&volume, records.len() as u64);
    let scanned = raw
        .scan_entries_with_options(scan_options(records.len() as u64))?
        .collect::<MftResult<Vec<_>>>()?;
    assert_eq!(scanned.len(), 1);
    assert!(scanned[0].has_unnamed_data);

    let mut stream = MftStream::from_reader(
        Cursor::new(records.concat()),
        stream_options()?.resolve_extensions(true),
    )?;
    let base = stream.read_record(0)?;
    assert_eq!(scanned[0], base.entry);
    let extension = stream.read_record(1)?;
    assert!(
        extension
            .diagnostics
            .iter()
            .any(|diagnostic| matches!(diagnostic.kind, MftDiagnosticKind::ExtensionRecordOnly))
    );
    Ok(())
}

#[test]
fn attribute_list_mismatch_preserves_iteration_policy_and_stream_evidence() -> MftResult<()> {
    for (
        reference_type,
        reference_sequence,
        reference_id,
        reference_name,
        reference_vcn,
        extension_base,
        extension_type,
        extension_id,
        extension_name,
    ) in [
        (0x80, 8, 2, "", 0, 7 << 48, 0x80, 2, ""), // Sequence mismatch.
        (0x80, 7, 2, "", 0, 6 << 48, 0x80, 2, ""), // Base-reference mismatch.
        (0x80, 7, 9, "", 0, 7 << 48, 0x80, 2, ""), // Attribute-id mismatch.
        (0x30, 7, 2, "", 0, 7 << 48, 0x80, 2, ""), // Attribute-type mismatch.
        (0x80, 7, 2, "wanted", 0, 7 << 48, 0x80, 2, "actual"), // Name mismatch.
        (0x80, 7, 2, "", 1, 7 << 48, 0x80, 2, ""), // Lowest-VCN mismatch.
    ] {
        let mismatched = attribute_list_reference(
            reference_type,
            1,
            reference_sequence,
            reference_id,
            reference_name,
            reference_vcn,
        );
        let records = vec![
            record(0, &[resident(0x20, 1, "", &mismatched)]),
            record(
                extension_base,
                &[resident(
                    extension_type,
                    extension_id,
                    extension_name,
                    b"unmatched",
                )],
            ),
            record(0, &[resident(0x80, 1, "", b"following")]),
        ];

        let volume = volume_from_records(&records)?;
        let raw = raw_mft(&volume, records.len() as u64);
        let scanned = raw
            .scan_entries_with_options(scan_options(records.len() as u64))?
            .collect::<MftResult<Vec<_>>>()?;
        assert_eq!(
            scanned
                .iter()
                .map(|entry| entry.record_number)
                .collect::<Vec<_>>(),
            [0, 2]
        );

        let mut stream = MftStream::from_reader(
            Cursor::new(records.concat()),
            stream_options()?.resolve_extensions(true),
        )?;
        let mut records = stream.iter_records();
        let base = records.next().unwrap()?;
        assert!(!base.entry.has_unnamed_data);
        assert!(
            base.diagnostics
                .iter()
                .any(|diagnostic| matches!(diagnostic.kind, MftDiagnosticKind::ExtensionMismatch))
        );
        assert_eq!(scanned[0], base.entry);
        let extension = records.next().unwrap()?;
        assert_eq!(extension.entry.record_number, 1);
        assert!(
            extension.diagnostics.iter().any(|diagnostic| matches!(
                diagnostic.kind,
                MftDiagnosticKind::ExtensionRecordOnly
            ))
        );
        assert_eq!(records.next().unwrap()?.entry.record_number, 2);
        assert!(records.next().is_none());
    }
    Ok(())
}

struct FailingReader {
    inner: Cursor<Vec<u8>>,
    fail_at: u64,
}

impl Read for FailingReader {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if self.inner.position() >= self.fail_at {
            return Err(io::Error::other("injected record read failure"));
        }
        self.inner.read(output)
    }
}

impl Seek for FailingReader {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.inner.seek(position)
    }
}

#[test]
fn main_io_errors_terminate_both_iterators() -> MftResult<()> {
    let records = [record(0, &[]), record(0, &[]), record(0, &[])];
    let live_source = Arc::new(SparseVolumeFixture::new(vec![
        (0, records[0].clone()),
        ((2 * RECORD_SIZE) as u64, records[2].clone()),
    ]));
    let volume = live_source.open()?;
    let raw = raw_mft(&volume, records.len() as u64);
    let mut entries = raw.scan_entries_with_options(scan_options(records.len() as u64))?;
    assert_eq!(entries.next().unwrap()?.record_number, 0);
    assert!(matches!(entries.next(), Some(Err(MftError::Io(_)))));
    assert!(entries.next().is_none());
    assert!(entries.next().is_none());

    let reader = FailingReader {
        inner: Cursor::new(records.concat()),
        fail_at: RECORD_SIZE as u64,
    };
    let mut stream = MftStream::from_reader(reader, stream_options()?)?;
    let mut records = stream.iter_records();
    assert_eq!(records.next().unwrap()?.entry.record_number, 0);
    assert!(matches!(records.next(), Some(Err(MftError::Io(_)))));
    assert!(records.next().is_none());
    assert!(records.next().is_none());
    Ok(())
}
