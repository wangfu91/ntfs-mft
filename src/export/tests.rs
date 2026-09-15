use super::*;
use crate::{
    AdsInfo, FileAttributes, Filetime, MftDiagnostic, MftDiagnosticKind, MftError, MftFieldState,
    MftStream, MftStreamOptions, RawMftLink, RawMftScanOptions,
};
use std::{ffi::OsString, io::Cursor};

fn fixture() -> crate::MftResult<crate::MftStreamRecord> {
    let bytes = include_bytes!("../../tests/fixtures/omerbenamram-mft/MFT");
    let mut stream =
        MftStream::from_reader(Cursor::new(bytes.as_slice()), MftStreamOptions::new(1024)?)?;
    stream.read_record(5)
}

fn item() -> crate::MftResult<ExportItem> {
    Ok(ExportItem::V1(ExportItemV1::Record(Box::new(
        ExportRecordV1::from_stream(&fixture()?),
    ))))
}

#[test]
fn schema_and_golden_contract() -> Result<(), Box<dyn std::error::Error>> {
    let schema = serde_json::to_value(schemars::schema_for!(ExportItem))?;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    #[cfg(ntfs_mft_differential)]
    let root = root
        .parent()
        .ok_or("differential harness must be inside the repository")?;
    let schema_path = root.join("docs/export-v1.schema.json");
    let golden_path = root.join("tests/fixtures/export-v1.jsonl");
    // Explicit opt-in only; ordinary tests never rewrite the contract.
    let error = ExportItem::V1(ExportItemV1::Error(
        (&MftError::FixupMismatch { number: 42 }).into(),
    ));
    let golden = format!("{}\n", serde_json::to_string(&error)?);
    if std::env::var_os("NTFS_MFT_UPDATE_EXPORT_SCHEMA").is_some() {
        std::fs::write(
            &schema_path,
            format!("{}\n", serde_json::to_string_pretty(&schema)?),
        )?;
        std::fs::write(&golden_path, &golden)?;
    }
    assert_eq!(
        schema,
        serde_json::from_slice::<serde_json::Value>(&std::fs::read(schema_path)?)?
    );
    assert_eq!(golden, std::fs::read_to_string(golden_path)?);
    let validator = jsonschema::validator_for(&schema)?;
    for value in [serde_json::to_value(item()?)?, serde_json::to_value(error)?] {
        assert!(
            validator.is_valid(&value),
            "{:?}",
            validator.iter_errors(&value).collect::<Vec<_>>()
        );
    }
    assert!(
        !validator.is_valid(&serde_json::json!({"schema_version":"2", "type":"error", "data":{}}))
    );
    let mut invalid = serde_json::to_value(item()?)?;
    invalid["data"]["record_number"] = serde_json::json!(9007199254740993u64);
    assert!(!validator.is_valid(&invalid));
    invalid["data"]["record_number"] = serde_json::json!("01");
    assert!(!validator.is_valid(&invalid));
    assert!(
        serde_json::from_value::<ExportItem>(
            serde_json::json!({"schema_version":"2", "type":"error", "data":{}})
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn references_timestamps_and_unknown_bits_roundtrip() -> Result<(), Box<dyn std::error::Error>> {
    let mut record = fixture()?;
    record.entry.file_reference = crate::NtfsFileReference::new(u64::MAX);
    record.entry.si_file_attributes = FileAttributes::from_bits_retain(0x8000_0001);
    record.entry.real_size = u64::MAX;
    record.entry.has_unnamed_data = true;
    record.entry.si_created = Filetime::new(u64::MAX);
    let dto = ExportRecordV1::from_stream(&record);
    assert_eq!(dto.source, "mft_stream");
    assert_eq!(dto.reference.raw, "18446744073709551615");
    assert_eq!(dto.reference.sequence_number, 65535);
    assert_eq!(dto.reference.record_number.as_str(), "281474976710655");
    assert_eq!(dto.file_attributes, 0x8000_0001);
    assert_eq!(
        dto.unnamed_stream.as_ref().map(|s| s.real_size.as_str()),
        Some("18446744073709551615")
    );
    assert_eq!(
        dto.standard_information_times.created.status,
        TimestampStatus::OutOfRange
    );
    assert_eq!(dto.standard_information_times.created.utc, None);
    assert_eq!(dto, serde_json::from_slice(&serde_json::to_vec(&dto)?)?);
    let cases = [
        (0, "1601-01-01T00:00:00.0000000Z"),
        (116444735999999999, "1969-12-31T23:59:59.9999999Z"),
        (116444736000000001, "1970-01-01T00:00:00.0000001Z"),
    ];
    for (raw, readable) in cases {
        let time = ExportTimestamp::from(Filetime::new(raw));
        assert_eq!(time.utc.as_deref(), Some(readable));
        assert_eq!(time.ticks, raw.to_string());
        assert_eq!(time, serde_json::from_str(&serde_json::to_string(&time)?)?);
    }
    Ok(())
}

#[test]
fn lossless_names_links_and_streams() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(windows)]
    let name = {
        use std::os::windows::ffi::OsStringExt;
        OsString::from_wide(&[0x61, 0xd800, 0x0a, 0x22])
    };
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    let name = {
        use std::os::unix::ffi::OsStringExt;
        OsString::from_vec(vec![b'a', 0xff, b'\n', b'"'])
    };
    let export = ExportName::from_os_str(&name);
    let decoded: ExportName = serde_json::from_str(&serde_json::to_string(&export)?)?;
    assert_eq!(export, decoded);
    #[cfg(windows)]
    assert_eq!(
        decoded.raw,
        NameUnits::WindowsUtf16(vec![0x61, 0xd800, 0x0a, 0x22])
    );
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    assert_eq!(
        decoded.raw,
        NameUnits::UnixBytes(vec![b'a', 0xff, b'\n', b'"'])
    );
    let name = crate::NtfsName::from_utf16(vec![0x61, 0xd800, 0x0a, 0x22]);
    let mut record = fixture()?;
    let entry = &mut record.entry;
    entry.file_name = name.clone();
    entry.links = vec![
        RawMftLink {
            parent_reference: crate::NtfsFileReference::try_from_parts(5, 7).unwrap(),
            namespace: entry.namespace,
            file_name: name.clone(),
        },
        RawMftLink {
            parent_reference: crate::NtfsFileReference::try_from_parts(6, 8).unwrap(),
            namespace: entry.namespace,
            file_name: "second".into(),
        },
    ]
    .into();
    entry.alternate_data_streams = vec![
        AdsInfo {
            name,
            real_size: 0,
            allocated_size: 0,
            is_resident: true,
        },
        AdsInfo {
            name: "other".into(),
            real_size: u64::MAX,
            allocated_size: 4096,
            is_resident: false,
        },
    ]
    .into();
    let dto = ExportRecordV1::from_stream(&record);
    assert_eq!(dto.name_fidelity, "ntfs_utf16_preserved");
    let units = NameUnits::WindowsUtf16(vec![0x61, 0xd800, 0x0a, 0x22]);
    assert_eq!(dto.selected_name.name.raw, units);
    assert_eq!(dto.links[0].name.raw, units);
    assert_eq!(dto.alternate_streams[0].name.as_ref().unwrap().raw, units);
    assert_eq!(dto.links.len(), 2);
    assert_eq!(dto.links[0].parent.sequence_number, 7);
    assert_eq!(dto.alternate_streams.len(), 2);
    assert_eq!(dto.alternate_streams[1].real_size, u64::MAX.to_string());
    assert_eq!(dto, serde_json::from_str(&serde_json::to_string(&dto)?)?);
    Ok(())
}

#[test]
fn offline_evidence_and_raw_collection_policy() -> Result<(), Box<dyn std::error::Error>> {
    let mut record = fixture()?;
    record.metadata.standard_information = MftFieldState::Absent;
    record.metadata.file_names = MftFieldState::Unknown;
    record.metadata.alternate_data_streams = MftFieldState::Present { complete: false };
    record.diagnostics.push(MftDiagnostic {
        record_number: 5,
        attribute_type: Some(0x20),
        kind: MftDiagnosticKind::ExtensionRead {
            reference: u64::MAX,
            error: MftError::FixupMismatch { number: 42 },
        },
    });
    let dto = ExportRecordV1::from_stream(&record);
    assert_eq!(dto.metadata.standard_information, FieldState::Absent);
    assert_eq!(dto.metadata.file_names, FieldState::Unknown);
    assert_eq!(dto.metadata.alternate_data_streams, FieldState::Partial);
    let diagnostic = dto.diagnostics.last().ok_or("missing diagnostic")?;
    assert_eq!(diagnostic.code, "extension_read");
    assert_eq!(
        diagnostic.reference.as_ref().map(|r| r.raw.as_str()),
        Some("18446744073709551615")
    );
    assert_eq!(
        diagnostic
            .error
            .as_ref()
            .and_then(|e| e.record_number.as_deref()),
        Some("42")
    );
    assert_eq!(dto.base_sequence_number, None);
    assert_eq!(dto.path, ExportPath::NotRequested);
    let options = RawMftScanOptions::builder()
        .collect_alternate_data_streams(false)
        .build();
    record.entry.collection_options = options.entry();
    record.entry.alternate_data_streams = Box::default();
    let raw = ExportRecordV1::from_entry(&record.entry);
    assert_eq!(raw.source, "entry");
    assert_eq!(
        raw.metadata.alternate_data_streams,
        FieldState::NotCollected
    );
    assert_eq!(raw.metadata.standard_information, FieldState::Unknown);
    assert!(raw.diagnostics.is_empty());
    record.entry.collection_options = crate::RawMftEntryOptions::default();
    let collected = ExportRecordV1::from_entry(&record.entry);
    assert_eq!(
        collected.metadata.alternate_data_streams,
        FieldState::NotObserved
    );
    #[cfg(feature = "jsonl")]
    {
        let mut bytes = Vec::new();
        JsonlWriter::new(&mut bytes).write_entry(&record.entry)?;
        let item: ExportItem = serde_json::from_slice(&bytes)?;
        assert_eq!(
            item,
            ExportItem::V1(ExportItemV1::Record(Box::new(collected)))
        );
    }

    Ok(())
}

#[cfg(feature = "jsonl")]
mod streaming {
    use super::*;
    use std::{
        cell::Cell,
        io::{self, Write},
    };

    #[test]
    fn full_fixture_output_validates_without_collecting_scan()
    -> Result<(), Box<dyn std::error::Error>> {
        struct ValidatorSink {
            validator: jsonschema::Validator,
            records: usize,
            errors: usize,
        }
        impl Write for ValidatorSink {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                if bytes.last() != Some(&b'\n') {
                    return Err(io::Error::other("missing LF"));
                }
                let value: serde_json::Value =
                    serde_json::from_slice(bytes).map_err(io::Error::other)?;
                self.validator
                    .validate(&value)
                    .map_err(|e| io::Error::other(e.to_string()))?;
                match value["type"].as_str() {
                    Some("record") => self.records += 1,
                    Some("error") => self.errors += 1,
                    _ => return Err(io::Error::other("unknown type")),
                }
                let decoded: ExportItem =
                    serde_json::from_value(value.clone()).map_err(io::Error::other)?;
                if serde_json::to_value(decoded).map_err(io::Error::other)? != value {
                    return Err(io::Error::other("roundtrip changed output"));
                }
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let schema = serde_json::to_value(schemars::schema_for!(ExportItem))?;
        let sink = ValidatorSink {
            validator: jsonschema::validator_for(&schema)?,
            records: 0,
            errors: 0,
        };
        let mut output = JsonlWriter::new(sink);
        let bytes = include_bytes!("../../tests/fixtures/omerbenamram-mft/MFT");
        let mut stream = MftStream::from_reader(
            Cursor::new(bytes.as_slice()),
            MftStreamOptions::new(1024)?.resolve_extensions(true),
        )?;
        for record in stream.iter_records() {
            match record {
                Ok(record) => output.write_stream(&record)?,
                Err(error) => output.write_error(&error)?,
            }
        }
        let sink = output.finish()?;
        assert_eq!((sink.records, sink.errors), (13068, 20));
        Ok(())
    }

    struct ShortSink {
        bytes: Vec<u8>,
        remaining: usize,
        flush_fails: bool,
    }
    impl Write for ShortSink {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self.remaining == 0 {
                return Err(io::Error::other("injected failure"));
            }
            let len = bytes.len().min(3).min(self.remaining);
            self.bytes.extend_from_slice(&bytes[..len]);
            self.remaining -= len;
            Ok(len)
        }
        fn flush(&mut self) -> io::Result<()> {
            if self.flush_fails {
                Err(io::Error::other("flush failure"))
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn writes_separate_lines_in_order_with_short_writes() -> Result<(), Box<dyn std::error::Error>>
    {
        let sink = ShortSink {
            bytes: Vec::new(),
            remaining: usize::MAX,
            flush_fails: false,
        };
        let mut writer = JsonlWriter::new(sink);
        writer.write_item(&item()?)?;
        writer.write_error(&MftError::InvalidRecordData("line\nbreak\r\n\"quote\""))?;
        writer.write_item(&item()?)?;
        let output = writer.finish()?.bytes;
        let lines: Vec<_> = output.split(|&b| b == b'\n').collect();
        assert_eq!(lines.len(), 4);
        assert!(lines[3].is_empty());
        assert_eq!(serde_json::from_slice::<ExportItem>(lines[0])?, item()?);
        assert!(matches!(
            serde_json::from_slice::<ExportItem>(lines[1])?,
            ExportItem::V1(ExportItemV1::Error(_))
        ));
        assert_eq!(lines[0], lines[2]);
        assert!(JsonlWriter::new(Vec::<u8>::new()).finish()?.is_empty());
        Ok(())
    }

    #[test]
    fn output_failure_stops_consumption_and_poison_prevents_append()
    -> Result<(), Box<dyn std::error::Error>> {
        let consumed = Cell::new(0);
        let sink = ShortSink {
            bytes: Vec::new(),
            remaining: 5,
            flush_fails: false,
        };
        let mut writer = JsonlWriter::new(sink);
        let values = (0..10000).inspect(|_| consumed.set(consumed.get() + 1));
        let result: crate::MftResult<()> = (|| {
            for _ in values {
                writer.write_error(&MftError::FixupMismatch { number: 1 })?;
            }
            Ok(())
        })();
        assert!(result.is_err());
        assert_eq!(consumed.get(), 1);
        assert!(writer.write_item(&item()?).is_err());
        assert!(writer.flush().is_err());
        let mut writer = JsonlWriter::new(ShortSink {
            bytes: Vec::new(),
            remaining: usize::MAX,
            flush_fails: true,
        });
        writer.write_item(&item()?)?;
        assert!(writer.flush().is_err());
        assert!(writer.write_item(&item()?).is_err());
        Ok(())
    }
}
