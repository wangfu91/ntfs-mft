//! Development-only comparison with the immutable upstream revision in Cargo.toml.
//! See ../README.md for normalization rules and deliberately different policies.

mod stream;
mod synthetic;

use super::layout::{attribute::for_each_attribute, record::FileRecord};
use mft::{attribute::header::ResidentialHeader, entry::MftEntry};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
const MFT: &[u8] = include_bytes!("../../tests/fixtures/omerbenamram-mft/MFT");
const RECORD_SIZE: usize = 1024;
const UNIX_EPOCH_TICKS: i128 = 116_444_736_000_000_000;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Value {
    StandardInfo([u64; 4]),
    FileName {
        parent: u64,
        namespace: u8,
        name: Vec<u16>,
        times: [u64; 4],
    },
    Data {
        name: Vec<u16>,
        size: u64,
        nonresident: Option<Nonresident>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Nonresident {
    allocated: u64,
    initialized: u64,
    first_vcn: u64,
    last_vcn: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Attribute {
    offset: usize,
    instance: u16,
    value: Value,
}

#[derive(Debug, PartialEq, Eq)]
struct Record {
    number: u64,
    sequence: u16,
    base: u64,
    flags: u16,
    links: u16,
    attributes: Vec<Attribute>,
}

fn units(bytes: &[u8]) -> Vec<u16> {
    assert!(bytes.len().is_multiple_of(2));
    bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .collect()
}

fn ours(number: u64, bytes: &[u8]) -> TestResult<Record> {
    let mut bytes = bytes.to_vec();
    let record = FileRecord::parse(number, None, &mut bytes)?;
    let mut attributes = Vec::new();
    let (start, end) = record.attrs_range();
    let mut offset = start;
    let mut failure = None;
    for_each_attribute(record.data, start, end, |attr| {
        let value = match attr.type_id() {
            0x10 => attr.as_standard_info().map(|h| {
                Value::StandardInfo([
                    h.creation_time,
                    h.modification_time,
                    h.mft_record_modification_time,
                    h.access_time,
                ])
            }),
            0x30 => attr.as_file_name().map(|(h, name)| Value::FileName {
                parent: h.parent_directory_reference,
                namespace: h.namespace,
                name: units(name),
                times: [
                    h.creation_time,
                    h.modification_time,
                    h.mft_record_modification_time,
                    h.access_time,
                ],
            }),
            0x80 => {
                let name = match attr.name_bytes() {
                    Some(bytes) => units(bytes),
                    None if !attr.has_name() => Vec::new(),
                    None => {
                        let instance = attr.header.id;
                        failure = Some(format!(
                            "record {number}, attribute {offset:#x}, instance {instance}: invalid DATA name range"
                        ));
                        offset += attr.len();
                        return;
                    }
                };
                if attr.is_non_resident() {
                    attr.nonresident_header().map(|h| Value::Data {
                        name,
                        size: h.data_size,
                        nonresident: Some(Nonresident {
                            allocated: h.allocated_size,
                            initialized: h.initialized_size,
                            first_vcn: h.lowest_vcn as u64,
                            last_vcn: h.highest_vcn as u64,
                        }),
                    })
                } else {
                    attr.resident_value().map(|value| Value::Data {
                        name,
                        size: value.len() as u64,
                        nonresident: None,
                    })
                }
            }
            _ => None,
        };
        if let Some(value) = value {
            attributes.push(Attribute {
                offset,
                instance: attr.header.id,
                value,
            });
        } else if matches!(attr.type_id(), 0x10 | 0x30 | 0x80) {
            failure = Some(format!(
                "record {number}, attribute {offset:#x}: local decoder returned None"
            ));
        }
        offset += attr.len();
    });
    if let Some(failure) = failure {
        return Err(failure.into());
    }
    Ok(Record {
        number: record.number,
        sequence: record.sequence_value(),
        base: record.base_reference(),
        flags: record.header.flags,
        links: record.link_count(),
        attributes,
    })
}

// Upstream timestamps have already lost the sub-microsecond digit. Convert the
// parsed timestamp back to ticks without strings, floating point, or timezone conversion.
macro_rules! times {
    ($h:expr) => {{
        [$h.created, $h.modified, $h.mft_modified, $h.accessed]
            .map(|t| u64::try_from(t.as_nanosecond() / 100 + UNIX_EPOCH_TICKS))
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?
            .try_into()
            .map_err(|_| "expected four timestamps")?
    }};
}

fn upstream(number: u64, bytes: &[u8]) -> TestResult<Record> {
    let entry = MftEntry::from_buffer(bytes.to_vec(), number)?;
    assert!(
        entry.header.is_valid(),
        "record {number}: upstream non-FILE header"
    );
    assert_eq!(
        entry.valid_fixup,
        Some(true),
        "record {number}: upstream fixup"
    );
    let mut attributes = Vec::new();
    // Filter payload decoders to the overlapping scope, but never discard their errors.
    for attr in entry.iter_attributes_matching(Some(vec![
        mft::attribute::MftAttributeType::StandardInformation,
        mft::attribute::MftAttributeType::FileName,
        mft::attribute::MftAttributeType::DATA,
    ])) {
        let attr = attr.map_err(|e| format!("record {number}: upstream attribute: {e}"))?;
        let value = if let Some(h) = attr.data.as_standard_info() {
            Value::StandardInfo(times!(h))
        } else if let Some(h) = attr.data.as_file_name() {
            Value::FileName {
                parent: h.parent.entry | (u64::from(h.parent.sequence) << 48),
                namespace: h.namespace as u8,
                name: units(h.name.as_utf16le_bytes()),
                times: times!(h),
            }
        } else {
            let name = units(attr.header.name.as_utf16le_bytes());
            match attr.header.residential_header {
                ResidentialHeader::Resident(h) => Value::Data {
                    name,
                    size: u64::from(h.data_size),
                    nonresident: None,
                },
                ResidentialHeader::NonResident(h) => Value::Data {
                    name,
                    size: h.file_size,
                    nonresident: Some(Nonresident {
                        allocated: h.allocated_length,
                        initialized: h.valid_data_length,
                        first_vcn: h.vnc_first,
                        last_vcn: h.vnc_last,
                    }),
                },
            }
        };
        attributes.push(Attribute {
            offset: attr.header.start_offset as usize,
            instance: attr.header.instance,
            value,
        });
    }
    let h = entry.header;
    Ok(Record {
        number: h.record_number,
        sequence: h.sequence,
        base: h.base_reference.entry | (u64::from(h.base_reference.sequence) << 48),
        flags: h.flags.bits(),
        links: h.hard_link_count,
        attributes,
    })
}

fn compare(number: u64, bytes: &[u8]) -> TestResult<Record> {
    let local = ours(number, bytes)?;
    let other = upstream(number, bytes)?;
    assert_eq!(
        (
            local.number,
            local.sequence,
            local.base,
            local.flags & 0xf,
            local.links
        ),
        (
            other.number,
            other.sequence,
            other.base,
            other.flags,
            other.links
        ),
        "record {number}: header"
    );
    assert_eq!(
        local.attributes.len(),
        other.attributes.len(),
        "record {number}: overlapping attribute count"
    );
    for (left, right) in local.attributes.iter().zip(&other.attributes) {
        let mut comparable = left.clone();
        match &mut comparable.value {
            Value::StandardInfo(times) | Value::FileName { times, .. } => {
                for time in times {
                    *time = *time / 10 * 10;
                }
            }
            Value::Data { .. } => {}
        }
        assert_eq!(
            &comparable, right,
            "record {number}, attribute {:#x}: normalized fields",
            left.offset
        );
    }
    Ok(local)
}

#[test]
fn shared_fixture_overlapping_fields() -> TestResult {
    assert_eq!(
        MFT.len(),
        13_402_112,
        "fixture missing: restore Git LFS object (see fixture README)"
    );
    let (mut files, mut empty, mut si, mut names, mut streams) = (0, 0, 0, 0, 0);
    for (number, bytes) in MFT.as_chunks::<RECORD_SIZE>().0.iter().enumerate() {
        if bytes[..4] == [0; 4] {
            assert!(ours(number as u64, bytes).is_err());
            let other = MftEntry::from_buffer(bytes.to_vec(), number as u64)?;
            assert!(!other.header.is_valid());
            assert_eq!(other.valid_fixup, None);
            empty += 1;
            continue;
        }
        let record = compare(number as u64, bytes)?;
        for attr in record.attributes {
            match attr.value {
                Value::StandardInfo(_) => si += 1,
                Value::FileName { .. } => names += 1,
                Value::Data { .. } => streams += 1,
            }
        }
        files += 1;
    }
    assert_eq!((files, empty), (13_068, 20));
    assert_eq!((si, names, streams), (13_064, 18_506, 12_089));
    eprintln!(
        "compared {files} FILE records: {si} SI, {names} FN, {streams} DATA attributes; {empty} empty slots"
    );
    Ok(())
}
