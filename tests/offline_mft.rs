//! Public offline API tests: no volume handles or elevated privileges.
use ntfs_mft::{
    MftError, MftResult, MftStream, MftStreamOptions, OfflineFieldState as State, OfflineIssue,
};
use std::io::{self, Cursor, Read, Seek, SeekFrom};

fn options() -> MftResult<MftStreamOptions> {
    MftStreamOptions::new(1024)
}

fn resident(kind: u32, id: u16, name: &str, value: &[u8]) -> Vec<u8> {
    let name: Vec<u8> = name.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let value_offset = (24 + name.len()).div_ceil(8) * 8;
    let len = (value_offset + value.len()).div_ceil(8) * 8;
    let mut attr = vec![0; len];
    attr[..4].copy_from_slice(&kind.to_le_bytes());
    attr[4..8].copy_from_slice(&(len as u32).to_le_bytes());
    attr[9] = (name.len() / 2) as u8;
    attr[10..12].copy_from_slice(&24u16.to_le_bytes());
    attr[14..16].copy_from_slice(&id.to_le_bytes());
    attr[16..20].copy_from_slice(&(value.len() as u32).to_le_bytes());
    attr[20..22].copy_from_slice(&(value_offset as u16).to_le_bytes());
    attr[24..24 + name.len()].copy_from_slice(&name);
    attr[value_offset..value_offset + value.len()].copy_from_slice(value);
    attr
}

fn nonresident(kind: u32, id: u16, lowest_vcn: i64) -> Vec<u8> {
    let mut attr = vec![0; 72];
    attr[..4].copy_from_slice(&kind.to_le_bytes());
    attr[4..8].copy_from_slice(&72u32.to_le_bytes());
    attr[8] = 1;
    attr[14..16].copy_from_slice(&id.to_le_bytes());
    attr[16..24].copy_from_slice(&lowest_vcn.to_le_bytes());
    attr[24..32].copy_from_slice(&lowest_vcn.to_le_bytes());
    attr[32..34].copy_from_slice(&64u16.to_le_bytes());
    attr[40..48].copy_from_slice(&4096u64.to_le_bytes());
    attr[48..56].copy_from_slice(&12u64.to_le_bytes());
    attr[56..64].copy_from_slice(&12u64.to_le_bytes());
    attr[64..68].copy_from_slice(&[0x11, 1, 50, 0]);
    attr
}

fn reference(kind: u32, slot: u64, sequence: u16, id: u16, name: &str, vcn: i64) -> Vec<u8> {
    let name: Vec<u8> = name.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let len = (26 + name.len()).div_ceil(8) * 8;
    let mut bytes = vec![0; len];
    bytes[..4].copy_from_slice(&kind.to_le_bytes());
    bytes[4..6].copy_from_slice(&(len as u16).to_le_bytes());
    bytes[6] = (name.len() / 2) as u8;
    bytes[7] = 26;
    bytes[8..16].copy_from_slice(&vcn.to_le_bytes());
    bytes[16..24].copy_from_slice(&(slot | ((sequence as u64) << 48)).to_le_bytes());
    bytes[24..26].copy_from_slice(&id.to_le_bytes());
    bytes[26..26 + name.len()].copy_from_slice(&name);
    bytes
}

fn record(size: usize, base_reference: u64, attributes: &[Vec<u8>]) -> Vec<u8> {
    let count = size / 512 + 1;
    let attrs_offset = (48 + count * 2).div_ceil(8) * 8;
    let mut bytes = vec![0; size];
    bytes[..4].copy_from_slice(b"FILE");
    bytes[4..6].copy_from_slice(&48u16.to_le_bytes());
    bytes[6..8].copy_from_slice(&(count as u16).to_le_bytes());
    bytes[16..18].copy_from_slice(&7u16.to_le_bytes());
    bytes[18..20].copy_from_slice(&1u16.to_le_bytes());
    bytes[20..22].copy_from_slice(&(attrs_offset as u16).to_le_bytes());
    bytes[22..24].copy_from_slice(&1u16.to_le_bytes());
    bytes[28..32].copy_from_slice(&(size as u32).to_le_bytes());
    bytes[32..40].copy_from_slice(&base_reference.to_le_bytes());
    let mut offset = attrs_offset;
    for attr in attributes {
        bytes[offset..offset + attr.len()].copy_from_slice(attr);
        offset += attr.len();
    }
    bytes[offset..offset + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    bytes[24..28].copy_from_slice(&((offset + 4) as u32).to_le_bytes());
    bytes[48..50].copy_from_slice(&0xabcd_u16.to_le_bytes());
    for sector in 1..count {
        let trailer = sector * 512 - 2;
        let old = [bytes[trailer], bytes[trailer + 1]];
        bytes[48 + sector * 2..50 + sector * 2].copy_from_slice(&old);
        bytes[trailer..trailer + 2].copy_from_slice(&0xabcd_u16.to_le_bytes());
    }
    bytes
}

#[test]
fn fixture_public_iteration_and_random_access() -> MftResult<()> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/omerbenamram-mft/MFT");
    let mut stream = MftStream::from_path(&path, options()?)?;
    assert_eq!(stream.record_count(), 13088);
    let expected = stream.read_record(0)?;
    assert_eq!(expected.entry.file_name, "$MFT");
    assert_eq!(expected.entry.real_size, 13_402_112);
    assert_eq!(
        expected.metadata.unnamed_data,
        State::Present { complete: true }
    );
    assert!(!expected.external_payloads.is_empty());
    let mut counts = (0, 0, 0, 0);
    for item in stream.iter_records() {
        match item {
            Ok(r) => {
                counts.0 += 1;
                counts.2 += usize::from(r.entry.is_used);
                counts.3 += usize::from(r.entry.base_record_reference != 0);
                assert_eq!(r.stream_offset, r.entry.record_number * 1024);
            }
            Err(e) => {
                assert!(e.is_parse_error());
                counts.1 += 1;
            }
        }
    }
    assert_eq!(counts, (13068, 20, 11839, 4));
    let bytes = std::fs::read(path)?;
    let mut memory = MftStream::from_reader(Cursor::new(bytes.as_slice()), options()?)?;
    assert_eq!(memory.read_record(0)?.entry, expected.entry);
    assert!(memory.read_record(13087).is_err());
    for n in [5, 42, 1, 4] {
        assert_eq!(memory.read_record(n)?.entry, stream.read_record(n)?.entry);
    }
    Ok(())
}

#[test]
fn geometry_empty_input_and_truncation() -> MftResult<()> {
    for size in [0, 1, 511, 513, 1536, 131072, usize::MAX] {
        assert!(MftStreamOptions::new(size).is_err());
    }
    for size in [512, 1024, 4096, 65536] {
        let bytes = record(size, 0, &[]);
        let mut cursor = Cursor::new(bytes);
        cursor.set_position(17);
        let mut stream = MftStream::from_reader(cursor, MftStreamOptions::new(size)?)?;
        assert_eq!(stream.file_record_size(), size);
        assert_eq!(stream.read_record(0)?.stream_offset, 0);
        assert_eq!(stream.into_inner().get_ref().len(), size);
    }
    let mut empty = MftStream::from_reader(Cursor::new(Vec::<u8>::new()), options()?)?;
    assert_eq!(empty.record_count(), 0);
    assert!(empty.iter_records().next().is_none());
    assert!(empty.read_record(u64::MAX).is_err());
    let partial = MftStream::from_reader(Cursor::new(vec![0; 1025]), options()?);
    assert!(matches!(
        partial,
        Err(MftError::TruncatedRecord {
            offset: 1024,
            needed: 1024,
            got: 1
        })
    ));
    let wrong_geometry = record(1024, 0, &[]);
    let mut stream =
        MftStream::from_reader(Cursor::new(wrong_geometry), MftStreamOptions::new(512)?)?;
    assert!(stream.read_record(0).is_err());
    Ok(())
}

#[test]
fn corrupt_slots_do_not_stop_iteration_and_fixup_is_not_reapplied() -> MftResult<()> {
    let valid = record(1024, 0, &[]);
    let mut torn = valid.clone();
    torn[510] ^= 1;
    let bytes = [vec![0; 1024], torn, valid.clone()].concat();
    let mut stream = MftStream::from_reader(Cursor::new(bytes), options()?)?;
    let mut iter = stream.iter_records();
    assert!(matches!(
        iter.next(),
        Some(Err(MftError::InvalidMftRecord { number: 0, .. }))
    ));
    assert!(matches!(
        iter.next(),
        Some(Err(MftError::FixupMismatch { number: 1 }))
    ));
    assert!(matches!(iter.next(), Some(Ok(_))));
    assert!(iter.next().is_none());
    assert_eq!(stream.read_record(2)?.entry, stream.read_record(2)?.entry);
    assert_eq!(&stream.into_inner().get_ref()[2048..], &valid);
    Ok(())
}

#[test]
fn absence_empty_data_and_unavailable_list_are_distinct() -> MftResult<()> {
    let bytes = [
        record(1024, 0, &[]),
        record(1024, 0, &[resident(0x80, 1, "", &[])]),
        record(1024, 0, &[nonresident(0x20, 1, 0)]),
        record(1024, 0, &[nonresident(0x80, 1, 0)]),
        record(1024, 0, &[nonresident(0xc0, 1, 0)]),
    ]
    .concat();
    let mut stream =
        MftStream::from_reader(Cursor::new(bytes), options()?.resolve_extensions(true))?;
    assert_eq!(stream.read_record(0)?.metadata.unnamed_data, State::Absent);
    let empty = stream.read_record(1)?;
    assert!(empty.entry.has_unnamed_data);
    assert_eq!(empty.entry.real_size, 0);
    assert_eq!(
        empty.metadata.unnamed_data,
        State::Present { complete: true }
    );
    let missing = stream.read_record(2)?;
    assert_eq!(missing.metadata.unnamed_data, State::Unknown);
    assert!(
        missing
            .diagnostics
            .iter()
            .any(|d| matches!(d.issue, OfflineIssue::ExternalPayloadUnavailable))
    );
    let external_data = stream.read_record(3)?;
    assert_eq!(
        external_data.metadata.unnamed_data,
        State::Present { complete: true }
    );
    assert_eq!(external_data.external_payloads[0].attribute_type, 0x80);
    assert_eq!(
        stream.read_record(4)?.metadata.reparse_point,
        State::Unknown
    );
    Ok(())
}

#[test]
fn resident_extensions_select_exact_attributes_and_preserve_cursor() -> MftResult<()> {
    let mut refs = reference(0x80, 1, 7, 2, "", 0);
    refs.extend(reference(0x80, 1, 7, 3, "ads", 0));
    refs.extend(reference(0x80, 1, 7, 3, "ads", 0)); // duplicate
    let bytes = [
        record(1024, 0, &[resident(0x20, 1, "", &refs)]),
        record(
            1024,
            7 << 48,
            &[
                resident(0x80, 2, "", &[]),
                resident(0x80, 3, "ads", b"abc"),
                resident(0x80, 4, "unreferenced", b"other"),
            ],
        ),
        record(1024, 0, &[resident(0x80, 1, "", b"next")]),
    ]
    .concat();
    let mut plain = MftStream::from_reader(Cursor::new(bytes.clone()), options()?)?;
    assert_eq!(plain.read_record(0)?.metadata.unnamed_data, State::Unknown);
    let mut stream =
        MftStream::from_reader(Cursor::new(bytes), options()?.resolve_extensions(true))?;
    let base = stream.read_record(0)?;
    assert!(base.entry.has_unnamed_data);
    assert_eq!(
        base.metadata.unnamed_data,
        State::Present { complete: true }
    );
    assert_eq!(base.entry.alternate_data_streams.len(), 1);
    assert_eq!(base.entry.alternate_data_streams[0].name, "ads");
    assert!(base.diagnostics.is_empty());
    let rows = stream.iter_records().collect::<MftResult<Vec<_>>>()?;
    assert_eq!(rows[2].entry.real_size, 4);
    assert!(
        rows[1]
            .diagnostics
            .iter()
            .any(|d| matches!(d.issue, OfflineIssue::ExtensionRecordOnly))
    );
    Ok(())
}

#[test]
fn missing_mismatched_and_recursive_extensions_remain_partial() -> MftResult<()> {
    for (slot, seq, base_ref, id) in [
        (2, 7, 7 << 48, 2),
        (1, 8, 7 << 48, 2),
        (1, 7, 6 << 48, 2),
        (1, 7, 7 << 48, 9),
    ] {
        let bytes = [
            record(
                1024,
                0,
                &[resident(
                    0x20,
                    1,
                    "",
                    &reference(0x80, slot, seq, id, "", 0),
                )],
            ),
            record(1024, base_ref, &[resident(0x80, 2, "", b"secret")]),
        ]
        .concat();
        let mut stream =
            MftStream::from_reader(Cursor::new(bytes), options()?.resolve_extensions(true))?;
        let base = stream.read_record(0)?;
        assert_eq!(base.metadata.unnamed_data, State::Unknown);
        assert!(!base.entry.has_unnamed_data);
        assert!(!base.diagnostics.is_empty());
    }
    let bytes = [
        record(
            1024,
            0,
            &[resident(0x20, 1, "", &reference(0x80, 1, 7, 2, "", 0))],
        ),
        record(
            1024,
            7 << 48,
            &[
                resident(0x80, 2, "", b"x"),
                resident(0x20, 3, "", &reference(0x20, 0, 7, 1, "", 0)),
            ],
        ),
    ]
    .concat();
    let mut stream =
        MftStream::from_reader(Cursor::new(bytes), options()?.resolve_extensions(true))?;
    let base = stream.read_record(0)?;
    assert_eq!(
        base.metadata.unnamed_data,
        State::Present { complete: false }
    );
    assert!(
        base.diagnostics
            .iter()
            .any(|d| matches!(d.issue, OfflineIssue::NestedAttributeList))
    );
    Ok(())
}

#[test]
fn malformed_attributes_never_imply_absence() -> MftResult<()> {
    let mut invalid_name = resident(0x80, 1, "named", b"x");
    invalid_name[10..12].copy_from_slice(&u16::MAX.to_le_bytes());
    let mut stream =
        MftStream::from_reader(Cursor::new(record(1024, 0, &[invalid_name])), options()?)?;
    let parsed = stream.read_record(0)?;
    assert!(!parsed.entry.has_unnamed_data);
    assert_eq!(parsed.metadata.unnamed_data, State::Unknown);
    let mut truncated_attributes = record(1024, 0, &[]);
    let start = u16::from_le_bytes([truncated_attributes[20], truncated_attributes[21]]) as usize;
    truncated_attributes[start..start + 4].copy_from_slice(&0x80u32.to_le_bytes());
    let mut stream = MftStream::from_reader(Cursor::new(truncated_attributes), options()?)?;
    assert_eq!(stream.read_record(0)?.metadata.file_names, State::Unknown);
    let mut stream = MftStream::from_reader(
        Cursor::new(record(1024, 0, &[resident(0x20, 1, "", &[1; 9])])),
        options()?.resolve_extensions(true),
    )?;
    assert!(
        stream
            .read_record(0)?
            .diagnostics
            .iter()
            .any(|d| matches!(d.issue, OfflineIssue::MalformedAttributeList))
    );
    Ok(())
}

#[test]
fn split_data_is_not_reported_as_a_complete_summary() -> MftResult<()> {
    let bytes = record(1024, 0, &[nonresident(0x80, 1, 0), nonresident(0x80, 2, 1)]);
    let mut stream = MftStream::from_reader(Cursor::new(bytes), options()?)?;
    let row = stream.read_record(0)?;
    assert_eq!(
        row.metadata.unnamed_data,
        State::Present { complete: false }
    );
    assert_eq!(row.entry.real_size, 12);
    assert!(
        row.diagnostics
            .iter()
            .any(|d| matches!(d.issue, OfflineIssue::SplitDataStream))
    );
    Ok(())
}

struct FailingReader {
    inner: Cursor<Vec<u8>>,
    fail_at: u64,
}
impl Read for FailingReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.inner.position() >= self.fail_at {
            return Err(io::Error::other("injected read failure"));
        }
        self.inner.read(buf)
    }
}
impl Seek for FailingReader {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.inner.seek(pos)
    }
}

#[test]
fn io_failure_is_reported_once_and_fuses_iteration() -> MftResult<()> {
    let bytes = [
        record(1024, 0, &[]),
        record(1024, 0, &[]),
        record(1024, 0, &[]),
    ]
    .concat();
    let reader = FailingReader {
        inner: Cursor::new(bytes),
        fail_at: 1024,
    };
    let mut stream = MftStream::from_reader(reader, options()?)?;
    let mut iter = stream.iter_records();
    assert!(matches!(iter.next(), Some(Ok(_))));
    assert!(matches!(iter.next(), Some(Err(MftError::Io(_)))));
    assert!(iter.next().is_none());
    assert!(iter.next().is_none());
    Ok(())
}

#[test]
fn extension_io_failure_is_preserved_and_stops_iteration() -> MftResult<()> {
    let bytes = [
        record(
            1024,
            0,
            &[resident(0x20, 1, "", &reference(0x80, 1, 7, 2, "", 0))],
        ),
        record(1024, 7 << 48, &[resident(0x80, 2, "", b"x")]),
    ]
    .concat();
    let reader = FailingReader {
        inner: Cursor::new(bytes),
        fail_at: 1024,
    };
    let mut stream = MftStream::from_reader(reader, options()?.resolve_extensions(true))?;
    let mut iter = stream.iter_records();
    let row = iter
        .next()
        .ok_or(MftError::InvalidRecordData("expected base"))??;
    assert_eq!(row.metadata.unnamed_data, State::Unknown);
    assert!(row.diagnostics.iter().any(|d| matches!(
        &d.issue,
        OfflineIssue::ExtensionRead {
            error: MftError::Io(_),
            ..
        }
    )));
    assert!(iter.next().is_none());
    Ok(())
}

#[test]
fn attribute_name_vcn_and_self_references_are_checked() -> MftResult<()> {
    for (name, vcn) in [("wrong", 0), ("ads", 1)] {
        let bytes = [
            record(
                1024,
                0,
                &[resident(0x20, 1, "", &reference(0x80, 1, 7, 2, name, vcn))],
            ),
            record(1024, 7 << 48, &[resident(0x80, 2, "ads", b"x")]),
        ]
        .concat();
        let mut stream =
            MftStream::from_reader(Cursor::new(bytes), options()?.resolve_extensions(true))?;
        let row = stream.read_record(0)?;
        assert!(row.entry.alternate_data_streams.is_empty());
        assert_eq!(row.metadata.alternate_data_streams, State::Unknown);
    }
    let bytes = record(
        1024,
        0,
        &[
            resident(0x20, 1, "", &reference(0x80, 0, 7, 2, "", 0)),
            resident(0x80, 2, "", b"self"),
        ],
    );
    let mut stream =
        MftStream::from_reader(Cursor::new(bytes), options()?.resolve_extensions(true))?;
    let row = stream.read_record(0)?;
    assert!(row.diagnostics.is_empty());
    assert_eq!(row.entry.real_size, 4);
    Ok(())
}

#[test]
fn selected_extension_file_name_uses_shared_builder() -> MftResult<()> {
    let mut value = vec![0; 66];
    value[..8].copy_from_slice(&((5u64 << 48) | 5).to_le_bytes());
    value[8..16].copy_from_slice(&123u64.to_le_bytes());
    value[64] = 4;
    value[65] = 1;
    value.extend("name".encode_utf16().flat_map(u16::to_le_bytes));
    let bytes = [
        record(
            1024,
            0,
            &[resident(0x20, 1, "", &reference(0x30, 1, 7, 2, "", 0))],
        ),
        record(1024, 7 << 48, &[resident(0x30, 2, "", &value)]),
    ]
    .concat();
    let mut stream =
        MftStream::from_reader(Cursor::new(bytes), options()?.resolve_extensions(true))?;
    let row = stream.read_record(0)?;
    assert_eq!(row.entry.file_name, "name");
    assert_eq!(row.entry.fn_created.raw(), 123);
    assert_eq!(row.entry.parent_reference, ntfs_mft::Fid::from_parts(5, 5));
    assert_eq!(row.metadata.file_names, State::Present { complete: true });
    Ok(())
}

#[test]
fn overlapping_attribute_names_and_payloads_are_rejected() -> MftResult<()> {
    let mut resident_overlap = resident(0x80, 1, "ads", b"payload");
    resident_overlap[20..22].copy_from_slice(&24u16.to_le_bytes());
    let mut nonresident_overlap = nonresident(0x80, 1, 0);
    nonresident_overlap[9] = 1;
    nonresident_overlap[10..12].copy_from_slice(&64u16.to_le_bytes());
    for attr in [resident_overlap, nonresident_overlap] {
        let mut stream = MftStream::from_reader(Cursor::new(record(1024, 0, &[attr])), options()?)?;
        let row = stream.read_record(0)?;
        assert!(row.entry.alternate_data_streams.is_empty());
        assert_eq!(row.metadata.alternate_data_streams, State::Unknown);
        assert!(
            row.diagnostics
                .iter()
                .any(|d| matches!(d.issue, OfflineIssue::MalformedAttribute))
        );
    }
    // A named runlist immediately after the name is valid (no required gap).
    let mut valid = nonresident(0x80, 1, 0);
    valid.resize(80, 0);
    valid[4..8].copy_from_slice(&80u32.to_le_bytes());
    valid[9] = 4;
    valid[10..12].copy_from_slice(&64u16.to_le_bytes());
    valid[32..34].copy_from_slice(&72u16.to_le_bytes());
    valid[64..72].copy_from_slice(&[b'n', 0, b'a', 0, b'm', 0, b'e', 0]);
    valid[72..76].copy_from_slice(&[0x11, 1, 50, 0]);
    let mut stream = MftStream::from_reader(Cursor::new(record(1024, 0, &[valid])), options()?)?;
    let row = stream.read_record(0)?;
    assert_eq!(row.entry.alternate_data_streams[0].name, "name");
    assert_eq!(
        row.metadata.alternate_data_streams,
        State::Present { complete: true }
    );
    assert!(row.diagnostics.is_empty());
    Ok(())
}

#[test]
fn impossible_initialized_size_is_not_complete_metadata() -> MftResult<()> {
    let mut attr = nonresident(0x80, 1, 0);
    attr[56..64].copy_from_slice(&13u64.to_le_bytes()); // logical size is 12
    let mut stream = MftStream::from_reader(Cursor::new(record(1024, 0, &[attr])), options()?)?;
    let row = stream.read_record(0)?;
    assert_eq!(
        row.metadata.unnamed_data,
        State::Present { complete: false }
    );
    assert!(
        row.diagnostics
            .iter()
            .any(|d| matches!(d.issue, OfflineIssue::MalformedAttribute))
    );
    let mut continuation = nonresident(0x80, 1, 1);
    continuation[56..64].copy_from_slice(&u64::MAX.to_le_bytes());
    let mut stream =
        MftStream::from_reader(Cursor::new(record(1024, 0, &[continuation])), options()?)?;
    let row = stream.read_record(0)?;
    assert!(
        !row.diagnostics
            .iter()
            .any(|d| matches!(d.issue, OfflineIssue::MalformedAttribute))
    );
    Ok(())
}

struct HugeStream;
impl Read for HugeStream {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        Err(io::Error::other("must reject before reading"))
    }
}
impl Seek for HugeStream {
    fn seek(&mut self, _: SeekFrom) -> io::Result<u64> {
        Ok(u64::MAX - 1023)
    }
}

#[test]
fn impossible_stream_length_is_rejected_before_allocation() -> MftResult<()> {
    assert!(matches!(
        MftStream::from_reader(HugeStream, options()?),
        Err(MftError::InvalidOptions(_))
    ));
    Ok(())
}

#[test]
fn offline_records_build_a_volume_independent_path_index() -> MftResult<()> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/omerbenamram-mft/MFT");
    let mut stream = MftStream::from_path(path, options()?)?;
    let mut index = ntfs_mft::MftPathIndex::new();
    for number in [5, 28, 29, 30] {
        index.insert(&stream.read_record(number)?.entry)?;
    }
    let entry = stream.read_record(30)?.entry;
    assert_eq!(
        index.resolve_strict(entry.file_reference)?,
        std::path::PathBuf::from("WINDOWS")
            .join("system32")
            .join("config")
    );
    Ok(())
}
