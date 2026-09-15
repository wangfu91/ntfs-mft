//! Iteration helpers for walking NTFS attribute records and `$ATTRIBUTE_LIST` entries.
//!
//! These helpers operate on already-bounded byte slices from a fixed-up FILE
//! record and stop cleanly on malformed trailing data instead of panicking or
//! reading past the validated region.

use std::mem::size_of;

use zerocopy::FromBytes;

use super::{AttributeListEntryHeader, NtfsAttribute, NtfsAttributeType};
use crate::raw_mft::layout::record::FileRecord;

/// Minimum byte length of an `$ATTRIBUTE_LIST` entry (no attribute name).
const ATTR_LIST_ENTRY_MIN_SIZE: usize = size_of::<AttributeListEntryHeader>();

/// Fully decoded identity of one `$ATTRIBUTE_LIST` entry.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct AttributeListReference {
    pub(crate) type_id: u32,
    pub(crate) file_reference: u64,
    pub(crate) id: u16,
    pub(crate) lowest_vcn: i64,
    pub(crate) name: Vec<u8>,
}

/// Best-effort decode result for a materialized `$ATTRIBUTE_LIST`.
pub(crate) struct AttributeListReferences {
    pub(crate) entries: Vec<AttributeListReference>,
    pub(crate) complete: bool,
}

impl AttributeListReference {
    pub(crate) fn record_number(&self) -> u64 {
        self.file_reference & 0x0000_ffff_ffff_ffff
    }

    pub(crate) fn sequence_number(&self) -> u16 {
        (self.file_reference >> 48) as u16
    }

    pub(crate) fn matches(&self, attr: &NtfsAttribute<'_>) -> bool {
        let name_matches = if self.name.is_empty() {
            attr.header.name_length == 0
        } else {
            attr.name_bytes() == Some(self.name.as_slice())
        };
        self.type_id == attr.type_id()
            && self.id == attr.header.id
            && name_matches
            && self.lowest_vcn
                == attr
                    .nonresident_header()
                    .map_or(0, |header| header.lowest_vcn)
    }

    /// Validate that a loaded FILE record is the exact source generation named
    /// by this list entry and belongs to the expected base record.
    pub(crate) fn matches_record(
        &self,
        record: &FileRecord<'_>,
        base_record_number: u64,
        base_file_reference: u64,
    ) -> bool {
        self.record_number() == record.number
            && self.sequence_number() == record.sequence_value()
            && if record.number == base_record_number {
                record.base_reference() == 0
            } else {
                record.base_reference() == base_file_reference
            }
    }
}

/// Decode a complete `$ATTRIBUTE_LIST`, including the fields needed to bind an
/// entry to one exact attribute in one exact FILE-record generation.
///
/// References decoded before the first malformed entry remain available, while
/// `complete` tells diagnostic callers that the list may have hidden more data.
/// A short all-zero alignment tail is accepted.
pub(crate) fn parse_attribute_list_references(data: &[u8]) -> AttributeListReferences {
    let mut references = Vec::new();
    let mut offset = 0usize;
    let mut complete = true;
    while offset < data.len() {
        let remaining = &data[offset..];
        if remaining.len() < 8 && remaining.iter().all(|byte| *byte == 0) {
            break;
        }
        let Some(header) = remaining
            .get(..ATTR_LIST_ENTRY_MIN_SIZE)
            .and_then(|bytes| AttributeListEntryHeader::read_from_bytes(bytes).ok())
        else {
            complete = false;
            break;
        };
        let len = header.record_length as usize;
        let name_start = header.attribute_name_offset as usize;
        let name_len = header.attribute_name_length as usize * 2;
        let Some(name_end) = name_start.checked_add(name_len) else {
            complete = false;
            break;
        };
        if len < ATTR_LIST_ENTRY_MIN_SIZE
            || len > remaining.len()
            || header.lowest_vcn < 0
            || (name_len != 0 && (name_start < ATTR_LIST_ENTRY_MIN_SIZE || name_end > len))
        {
            complete = false;
            break;
        }
        references.push(AttributeListReference {
            type_id: header.type_id,
            file_reference: header.file_reference,
            id: header.attribute_id,
            lowest_vcn: header.lowest_vcn,
            name: if name_len == 0 {
                Vec::new()
            } else {
                remaining[name_start..name_end].to_vec()
            },
        });
        offset += len;
    }
    AttributeListReferences {
        entries: references,
        complete,
    }
}

/// Iterate attributes in a FILE record buffer starting at `attrs_offset`
/// up to `used_size`. Skips invalid trailing data; stops at the End
/// marker.
pub(crate) fn for_each_attribute<'a, F>(
    data: &'a [u8],
    attrs_offset: usize,
    used_size: usize,
    mut f: F,
) where
    F: FnMut(&NtfsAttribute<'a>),
{
    let _ = for_each_attribute_checked(data, attrs_offset, used_size, &mut f);
}

/// Walk attributes and report whether an end marker was reached.
/// The best-effort walker intentionally discards this diagnostic.
pub(crate) fn for_each_attribute_checked<'a, F>(
    data: &'a [u8],
    attrs_offset: usize,
    used_size: usize,
    mut f: F,
) -> bool
where
    F: FnMut(&NtfsAttribute<'a>),
{
    let used = used_size.min(data.len());
    let mut offset = attrs_offset;
    while offset + 4 <= used {
        // Peek at type_id first so we can detect the End marker (which
        // typically carries length=0 and would otherwise be rejected by
        // `NtfsAttribute::new`).
        let type_id = u32::from_le_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ]);
        if type_id == NtfsAttributeType::End as u32 {
            return true;
        }
        let slice = &data[offset..used];
        let attr = match NtfsAttribute::new(slice) {
            Some(a) => a,
            None => break,
        };
        f(&attr);
        let len = attr.len();
        if len == 0 {
            break;
        }
        offset = match offset.checked_add(len) {
            Some(n) if n <= used => n,
            _ => break,
        };
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use zerocopy::IntoBytes;

    use crate::raw_mft::layout::{
        attribute::{NtfsAttributeHeader, NtfsResidentAttributeHeader},
        record::FileRecordHeader,
    };

    fn build_resident_attr(type_id: u32, value: &[u8]) -> Vec<u8> {
        let header_size = size_of::<NtfsResidentAttributeHeader>();
        let total = header_size + value.len();
        let mut buf = vec![0u8; total];
        let header = NtfsResidentAttributeHeader {
            attribute_header: NtfsAttributeHeader {
                type_id,
                length: total as u32,
                is_non_resident: 0,
                name_length: 0,
                name_offset: 0,
                flags: 0,
                id: 0,
            },
            value_length: value.len() as u32,
            value_offset: header_size as u16,
            indexed_flag: 0,
            _pad: 0,
        };
        buf[..header_size].copy_from_slice(header.as_bytes());
        buf[header_size..].copy_from_slice(value);
        buf
    }

    #[test]
    fn iterates_attributes_until_end() {
        let mut buf = vec![0u8; 512];
        let attrs_offset = 0usize;
        let a1 = build_resident_attr(NtfsAttributeType::StandardInformation as u32, &[0u8; 48]);
        buf[..a1.len()].copy_from_slice(&a1);
        let mut next = a1.len();
        let a2 = build_resident_attr(NtfsAttributeType::Data as u32, &[1, 2, 3, 4]);
        buf[next..next + a2.len()].copy_from_slice(&a2);
        next += a2.len();
        buf[next..next + 4].copy_from_slice(&(NtfsAttributeType::End as u32).to_le_bytes());

        let mut seen = Vec::new();
        for_each_attribute(&buf, attrs_offset, 512, |a| seen.push(a.type_id()));
        assert_eq!(
            seen,
            vec![
                NtfsAttributeType::StandardInformation as u32,
                NtfsAttributeType::Data as u32,
            ]
        );
    }

    #[test]
    fn decodes_attribute_list_entry() {
        let header = AttributeListEntryHeader {
            type_id: NtfsAttributeType::FileName as u32,
            record_length: size_of::<AttributeListEntryHeader>() as u16,
            attribute_name_length: 0,
            attribute_name_offset: 0,
            lowest_vcn: 0,
            file_reference: 0x0001_0000_0000_0042,
            attribute_id: 7,
        };
        let mut bytes = vec![0u8; size_of::<AttributeListEntryHeader>()];
        bytes.copy_from_slice(header.as_bytes());

        let decoded = parse_attribute_list_references(&bytes);
        let seen = decoded
            .entries
            .into_iter()
            .map(|reference| (reference.type_id, reference.file_reference))
            .collect::<Vec<_>>();

        assert_eq!(
            seen,
            vec![(NtfsAttributeType::FileName as u32, 0x0001_0000_0000_0042)]
        );
    }

    fn attr_list_entry(type_id: u32, file_reference: u64) -> Vec<u8> {
        let min = size_of::<AttributeListEntryHeader>();
        let header = AttributeListEntryHeader {
            type_id,
            record_length: min as u16,
            attribute_name_length: 0,
            attribute_name_offset: 0,
            lowest_vcn: 0,
            file_reference,
            attribute_id: 0,
        };
        header.as_bytes().to_vec()
    }

    #[test]
    fn decodes_multiple_attribute_list_entries() {
        let mut bytes = attr_list_entry(NtfsAttributeType::StandardInformation as u32, 0x11);
        bytes.extend(attr_list_entry(NtfsAttributeType::Data as u32, 0x22));

        let decoded = parse_attribute_list_references(&bytes);
        let seen = decoded
            .entries
            .into_iter()
            .map(|reference| (reference.type_id, reference.file_reference))
            .collect::<Vec<_>>();

        assert_eq!(
            seen,
            vec![
                (NtfsAttributeType::StandardInformation as u32, 0x11),
                (NtfsAttributeType::Data as u32, 0x22),
            ]
        );
    }

    #[test]
    fn attr_list_decoder_stops_at_zero_length_entry() {
        let mut bytes = attr_list_entry(NtfsAttributeType::FileName as u32, 0x42);
        // A second entry that declares record_length = 0 must halt iteration.
        let mut malformed = attr_list_entry(NtfsAttributeType::Data as u32, 0x99);
        malformed[4..6].copy_from_slice(&0u16.to_le_bytes()); // record_length = 0
        bytes.extend(malformed);

        let decoded = parse_attribute_list_references(&bytes);
        let seen = decoded
            .entries
            .into_iter()
            .map(|reference| (reference.type_id, reference.file_reference))
            .collect::<Vec<_>>();

        assert_eq!(seen, vec![(NtfsAttributeType::FileName as u32, 0x42)]);
        assert!(!decoded.complete);
    }

    #[test]
    fn exact_reference_decoder_retains_valid_prefix_and_marks_malformed_tail() {
        let mut bytes = attr_list_entry(NtfsAttributeType::FileName as u32, 0x42);
        let mut malformed = attr_list_entry(NtfsAttributeType::Data as u32, 0x99);
        malformed[4..6].copy_from_slice(&0u16.to_le_bytes());
        bytes.extend(malformed);

        let decoded = parse_attribute_list_references(&bytes);

        assert!(!decoded.complete);
        assert_eq!(decoded.entries.len(), 1);
        assert_eq!(decoded.entries[0].record_number(), 0x42);
        assert_eq!(
            decoded.entries[0].type_id,
            NtfsAttributeType::FileName as u32
        );
    }

    #[test]
    fn reference_source_validation_checks_record_sequence_and_base() {
        let record = FileRecord {
            data: &[],
            header: FileRecordHeader {
                signature: *b"FILE",
                update_sequence_offset: 0,
                update_sequence_length: 0,
                logfile_sequence_number: 0,
                sequence_value: 7,
                link_count: 0,
                attributes_offset: 0,
                flags: 0,
                used_size: 0,
                allocated_size: 0,
                base_reference: 7 << 48,
                next_attribute_id: 0,
            },
            number: 1,
        };
        let reference = AttributeListReference {
            type_id: NtfsAttributeType::Data as u32,
            file_reference: 1 | (7 << 48),
            id: 2,
            lowest_vcn: 0,
            name: Vec::new(),
        };

        assert!(reference.matches_record(&record, 0, 7 << 48));
        assert!(!reference.matches_record(&record, 0, 6 << 48));

        let wrong_sequence = AttributeListReference {
            file_reference: 1 | (8 << 48),
            ..reference.clone()
        };
        assert!(!wrong_sequence.matches_record(&record, 0, 7 << 48));

        let wrong_record = AttributeListReference {
            file_reference: 2 | (7 << 48),
            ..reference
        };
        assert!(!wrong_record.matches_record(&record, 0, 7 << 48));
    }

    #[test]
    fn attribute_iteration_respects_used_size() {
        let mut buf = vec![0u8; 512];
        let a1 = build_resident_attr(NtfsAttributeType::StandardInformation as u32, &[0u8; 48]);
        buf[..a1.len()].copy_from_slice(&a1);
        let a2 = build_resident_attr(NtfsAttributeType::Data as u32, &[1, 2, 3, 4]);
        buf[a1.len()..a1.len() + a2.len()].copy_from_slice(&a2);

        // used_size covers only the first attribute, so the second must be
        // ignored even though it is present in the buffer.
        let mut seen = Vec::new();
        for_each_attribute(&buf, 0, a1.len(), |a| seen.push(a.type_id()));
        assert_eq!(seen, vec![NtfsAttributeType::StandardInformation as u32]);
    }
}
