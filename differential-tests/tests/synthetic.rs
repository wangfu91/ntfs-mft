//! Byte-authored records: no production layout structs or serializers construct expectations.
use super::*;
use crate::raw_mft::entry_build::{EntryBuildOptions, RawMftEntry};

fn put16(b: &mut [u8], at: usize, value: u16) {
    b[at..at + 2].copy_from_slice(&value.to_le_bytes());
}
fn put32(b: &mut [u8], at: usize, value: u32) {
    b[at..at + 4].copy_from_slice(&value.to_le_bytes());
}
fn put64(b: &mut [u8], at: usize, value: u64) {
    b[at..at + 8].copy_from_slice(&value.to_le_bytes());
}
fn utf16(name: &[u16]) -> Vec<u8> {
    name.iter().flat_map(|u| u.to_le_bytes()).collect()
}
fn text(name: &str) -> Vec<u16> {
    name.encode_utf16().collect()
}

fn resident(kind: u32, name: &[u16], value: &[u8]) -> Vec<u8> {
    let start = (24 + name.len() * 2).next_multiple_of(8);
    let mut b = vec![0; (start + value.len()).next_multiple_of(8)];
    let len = b.len() as u32;
    put32(&mut b, 0, kind);
    put32(&mut b, 4, len);
    b[9] = name.len() as u8;
    put16(&mut b, 10, 24);
    put32(&mut b, 16, value.len() as u32);
    put16(&mut b, 20, start as u16);
    b[24..24 + name.len() * 2].copy_from_slice(&utf16(name));
    b[start..start + value.len()].copy_from_slice(value);
    b
}

const TIMES: [u64; 4] = [
    0,
    116_444_735_999_999_999,
    128_276_814_522_523_952,
    133_000_000_000_000_007,
];
fn si(length: usize) -> Vec<u8> {
    let mut value = vec![0; length];
    for (i, t) in TIMES.iter().enumerate() {
        put64(&mut value, i * 8, *t);
    }
    resident(0x10, &[], &value)
}

fn filename(name: &[u16], namespace: u8, parent: u64) -> Vec<u8> {
    let mut value = vec![0; 66 + name.len() * 2];
    put64(&mut value, 0, parent);
    for (i, t) in TIMES.iter().enumerate() {
        put64(&mut value, 8 + i * 8, *t);
    }
    value[64] = name.len() as u8;
    value[65] = namespace;
    value[66..].copy_from_slice(&utf16(name));
    resident(0x30, &[], &value)
}

fn nonresident(name: &[u16], first_vcn: u64) -> Vec<u8> {
    let runs = (64 + name.len() * 2).next_multiple_of(8);
    let mut b = vec![0; (runs + 4).next_multiple_of(8)];
    let len = b.len() as u32;
    put32(&mut b, 0, 0x80);
    put32(&mut b, 4, len);
    b[8] = 1;
    b[9] = name.len() as u8;
    put16(&mut b, 10, 64);
    put64(&mut b, 16, first_vcn);
    put64(&mut b, 24, first_vcn + 1);
    put16(&mut b, 32, runs as u16);
    put64(&mut b, 40, 8192);
    put64(&mut b, 48, 6001);
    put64(&mut b, 56, 5003);
    b[64..64 + name.len() * 2].copy_from_slice(&utf16(name));
    b[runs..runs + 4].copy_from_slice(&[0x11, 2, 42, 0]);
    b
}

fn record(attrs: Vec<Vec<u8>>, flags: u16, base: u64) -> Vec<u8> {
    let mut b = vec![0; RECORD_SIZE];
    b[..4].copy_from_slice(b"FILE");
    put16(&mut b, 4, 48);
    put16(&mut b, 6, 3);
    put16(&mut b, 16, 0xabcd);
    put16(&mut b, 18, 2);
    put16(&mut b, 20, 56);
    put16(&mut b, 22, flags);
    put32(&mut b, 28, RECORD_SIZE as u32);
    put64(&mut b, 32, base);
    let mut offset = 56;
    for (id, mut attr) in attrs.into_iter().enumerate() {
        put16(&mut attr, 14, id as u16);
        b[offset..offset + attr.len()].copy_from_slice(&attr);
        offset += attr.len();
    }
    put32(&mut b, offset, u32::MAX);
    put32(&mut b, 24, (offset + 4) as u32);
    put16(&mut b, 48, 0xbabe);
    for sector in 1..=2 {
        let at = sector * 512 - 2;
        let saved = [b[at], b[at + 1]];
        b[48 + sector * 2..50 + sector * 2].copy_from_slice(&saved);
        put16(&mut b, at, 0xbabe);
    }
    b
}

#[test]
fn timestamps_keep_local_100ns_and_upstream_microsecond_precision() -> TestResult {
    for length in [48, 72] {
        let bytes = record(
            vec![
                si(length),
                filename(&text("time"), 1, 0x1234_0000_0000_0005),
            ],
            1,
            0,
        );
        let parsed = compare(42, &bytes)?;
        assert_eq!(parsed.attributes[0].value, Value::StandardInfo(TIMES));
        let Value::FileName { times, .. } = parsed.attributes[1].value else {
            return Err("missing FN".into());
        };
        assert_eq!(times, TIMES);
        let other = upstream(42, &bytes)?;
        assert_eq!(
            other.attributes[0].value,
            Value::StandardInfo(TIMES.map(|t| t / 10 * 10))
        );
    }
    Ok(())
}

#[test]
fn all_names_namespaces_parents_and_duplicate_names_survive() -> TestResult {
    let names = [
        text("posix"),
        text("文件😀"),
        text("SHORT~1"),
        vec![0x61, 0xd800, 0x62],
    ];
    let attrs = names
        .iter()
        .enumerate()
        .map(|(ns, name)| filename(name, ns as u8, ((ns as u64 + 7) << 48) | 5))
        .collect();
    let parsed = compare(0x1234_5678, &record(attrs, 3, 0))?;
    assert_eq!(parsed.sequence, 0xabcd);
    assert_eq!(parsed.number, 0x1234_5678);
    for (ns, attr) in parsed.attributes.iter().enumerate() {
        assert_eq!(
            attr.value,
            Value::FileName {
                parent: ((ns as u64 + 7) << 48) | 5,
                namespace: ns as u8,
                name: names[ns].clone(),
                times: TIMES
            }
        );
    }
    let duplicate = filename(&text("same"), 1, 5);
    let parsed = compare(42, &record(vec![duplicate.clone(), duplicate], 1, 0))?;
    assert_eq!(parsed.attributes.len(), 2);
    assert_eq!(parsed.attributes[0].value, parsed.attributes[1].value);
    Ok(())
}

#[test]
fn data_sizes_residency_and_extension_headers() -> TestResult {
    let bytes = record(
        vec![
            resident(0x80, &[], &[]),
            resident(0x80, &text("small"), b"abc"),
            nonresident(&text("流😀"), 0),
        ],
        1,
        0,
    );
    let parsed = compare(42, &bytes)?;
    assert_eq!(
        parsed.attributes[0].value,
        Value::Data {
            name: vec![],
            size: 0,
            nonresident: None
        }
    );
    assert_eq!(
        parsed.attributes[1].value,
        Value::Data {
            name: text("small"),
            size: 3,
            nonresident: None
        }
    );
    assert_eq!(
        parsed.attributes[2].value,
        Value::Data {
            name: text("流😀"),
            size: 6001,
            nonresident: Some(Nonresident {
                allocated: 8192,
                initialized: 5003,
                first_vcn: 0,
                last_vcn: 1
            })
        }
    );
    let base = 0x1234_0000_0000_0018;
    let parsed = compare(43, &record(vec![nonresident(&[], 2)], 0, base))?;
    assert_eq!(parsed.base, base);
    assert_eq!(parsed.flags, 0);
    assert_eq!(
        parsed.attributes[0].value,
        Value::Data {
            name: vec![],
            size: 6001,
            nonresident: Some(Nonresident {
                allocated: 8192,
                initialized: 5003,
                first_vcn: 2,
                last_vcn: 3
            })
        }
    );
    Ok(())
}

#[test]
fn primary_name_selection_is_a_separate_policy() -> TestResult {
    for reverse in [false, true] {
        let mut attrs = vec![
            filename(&text("win32"), 1, 5),
            filename(&text("combined"), 3, 6),
        ];
        if reverse {
            attrs.reverse();
        }
        let bytes = record(attrs, 1, 0);
        compare(42, &bytes)?;
        let mut local_bytes = bytes.clone();
        let file = FileRecord::parse(42, None, &mut local_bytes)?;
        let local = RawMftEntry::from_record_with_attr_list(&file, EntryBuildOptions::full()).0;
        let other = MftEntry::from_buffer(bytes, 42)?;
        let selected = other
            .find_best_name_attribute()
            .ok_or("missing upstream primary name")?;
        assert_eq!(local.file_name, "combined");
        assert_eq!(
            selected.name.to_utf8_string(),
            if reverse { "combined" } else { "win32" }
        );
    }
    Ok(())
}

#[test]
fn unknown_record_flags_are_not_silently_treated_as_equal() -> TestResult {
    let bytes = record(vec![], 0x800f, 0);
    let local = compare(42, &bytes)?;
    assert_eq!(local.flags, 0x800f);
    assert_eq!(upstream(42, &bytes)?.flags, 0xf);
    Ok(())
}

#[test]
fn damaged_fixup_and_truncated_records_have_explicit_outcomes() -> TestResult {
    let valid = record(vec![si(72)], 1, 0);
    for sector in [1, 2] {
        let mut damaged = valid.clone();
        damaged[sector * 512 - 2] ^= 1;
        let other = MftEntry::from_buffer(damaged.clone(), 42)?;
        assert_eq!(other.valid_fixup, Some(false));
        assert!(matches!(
            FileRecord::parse(42, None, &mut damaged),
            Err(crate::MftError::FixupMismatch { .. })
        ));
    }
    for length in [0, 4, 41] {
        assert!(ours(42, &valid[..length]).is_err(), "local length {length}");
        assert!(
            MftEntry::from_buffer(valid[..length].to_vec(), 42).is_err(),
            "upstream length {length}"
        );
    }
    for length in [512, 1023] {
        assert!(ours(42, &valid[..length]).is_err(), "local length {length}");
        let other = MftEntry::from_buffer(valid[..length].to_vec(), 42)?;
        assert_eq!(other.valid_fixup, Some(false), "upstream length {length}");
    }
    Ok(())
}

#[test]
fn malformed_attribute_is_local_stop_and_upstream_error() -> TestResult {
    for bad_length in [0, 4096] {
        let mut bytes = record(vec![si(72)], 1, 0);
        put32(&mut bytes, 60, bad_length);
        assert!(ours(42, &bytes)?.attributes.is_empty());
        let other = MftEntry::from_buffer(bytes, 42)?;
        assert!(
            other
                .iter_attributes()
                .next()
                .ok_or("missing error")?
                .is_err()
        );
    }
    Ok(())
}

#[test]
fn malformed_data_name_is_not_normalized_to_an_unnamed_stream() -> TestResult {
    for mut attr in [
        resident(0x80, &text("named"), b"x"),
        nonresident(&text("named"), 0),
    ] {
        // NameLength remains nonzero, but NameOffset cannot address this attribute.
        put16(&mut attr, 10, u16::MAX);
        let bytes = record(vec![attr], 1, 0);
        let other = MftEntry::from_buffer(bytes.clone(), 42)?;
        assert!(
            other
                .iter_attributes()
                .next()
                .ok_or("missing attribute")?
                .is_err()
        );
        let error = ours(42, &bytes)
            .err()
            .ok_or("invalid counted name became an unnamed stream")?;
        assert_eq!(
            error.to_string(),
            "record 42, attribute 0x38, instance 0: invalid DATA name range"
        );
    }
    Ok(())
}

#[test]
fn attribute_name_nul_termination_is_an_explicit_representation_difference() -> TestResult {
    let bytes = record(vec![resident(0x80, &[0x61, 0, 0x62], b"x")], 1, 0);
    assert_eq!(
        ours(42, &bytes)?.attributes[0].value,
        Value::Data {
            name: vec![0x61, 0, 0x62],
            size: 1,
            nonresident: None
        }
    );
    assert_eq!(
        upstream(42, &bytes)?.attributes[0].value,
        Value::Data {
            name: vec![0x61],
            size: 1,
            nonresident: None
        }
    );
    Ok(())
}
