//! Shared FILE-record attribute dispatch for raw-MFT entry builders.

use crate::raw_mft::layout::{
    attribute::{NtfsAttribute, NtfsAttributeType, for_each_attribute},
    record::FileRecord,
};

use super::{capture::capture_attribute_list, entry::AttributeListInfo};

/// Consumer of typed NTFS attributes from one FILE record.
pub(super) trait AttributeConsumer {
    /// Fold a `$STANDARD_INFORMATION` attribute.
    fn on_standard_information(&mut self, attr: &NtfsAttribute<'_>);

    /// Fold a `$FILE_NAME` attribute.
    fn on_file_name(&mut self, attr: &NtfsAttribute<'_>);

    /// Fold a `$DATA` attribute.
    fn on_data(&mut self, attr: &NtfsAttribute<'_>);

    /// Fold a `$REPARSE_POINT` attribute.
    fn on_reparse_point(&mut self, attr: &NtfsAttribute<'_>);

    /// Capture a `$ATTRIBUTE_LIST` payload for later enrichment.
    fn on_attribute_list(&mut self, attr_list: AttributeListInfo);
}

/// Walk the attributes of a parsed FILE record and dispatch the attribute
/// types used by raw-MFT entry construction.
pub(super) fn fold_record_attributes<C>(record: &FileRecord<'_>, consumer: &mut C)
where
    C: AttributeConsumer,
{
    let (attrs_off, used) = record.attrs_range();
    for_each_attribute(record.data, attrs_off, used, |attr| {
        // Continuation headers do not carry authoritative stream sizes. Leave
        // missing initial segments available for attribute-list enrichment.
        let has_stream_sizes = attr.type_id() != NtfsAttributeType::Data as u32
            || attr
                .nonresident_header()
                .is_none_or(|header| header.lowest_vcn == 0);
        if has_stream_sizes && crate::raw_mft::inspection::attribute_shape_is_valid(attr) {
            fold_attribute(attr, consumer);
        }
    });
}

/// Dispatch one selected attribute, including attributes from validated extensions.
pub(super) fn fold_attribute<C: AttributeConsumer>(attr: &NtfsAttribute<'_>, consumer: &mut C) {
    let type_id = attr.type_id();
    if type_id == NtfsAttributeType::StandardInformation as u32 {
        consumer.on_standard_information(attr);
    } else if type_id == NtfsAttributeType::FileName as u32 {
        consumer.on_file_name(attr);
    } else if type_id == NtfsAttributeType::Data as u32 {
        consumer.on_data(attr);
    } else if type_id == NtfsAttributeType::ReparsePoint as u32 {
        consumer.on_reparse_point(attr);
    } else if type_id == NtfsAttributeType::AttributeList as u32
        && let Some(attr_list) = capture_attribute_list(attr)
    {
        consumer.on_attribute_list(attr_list);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raw_mft::entry_build::{EntryBuildOptions, RawMftBatchScratch, RawMftEntry};

    fn resident_attribute(type_id: u32, value: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0; (24 + value.len()).div_ceil(8) * 8];
        let length = bytes.len() as u32;
        bytes[..4].copy_from_slice(&type_id.to_le_bytes());
        bytes[4..8].copy_from_slice(&length.to_le_bytes());
        bytes[16..20].copy_from_slice(&(value.len() as u32).to_le_bytes());
        bytes[20..22].copy_from_slice(&24u16.to_le_bytes());
        bytes[24..24 + value.len()].copy_from_slice(value);
        bytes
    }

    fn record_bytes(attributes: &[Vec<u8>]) -> Vec<u8> {
        let mut bytes = vec![0; 1024];
        bytes[..4].copy_from_slice(b"FILE");
        bytes[4..6].copy_from_slice(&42u16.to_le_bytes());
        bytes[6..8].copy_from_slice(&3u16.to_le_bytes());
        bytes[16..18].copy_from_slice(&1u16.to_le_bytes());
        bytes[20..22].copy_from_slice(&56u16.to_le_bytes());
        bytes[22..24].copy_from_slice(&1u16.to_le_bytes());
        bytes[28..32].copy_from_slice(&1024u32.to_le_bytes());
        let mut offset = 56;
        for attribute in attributes {
            bytes[offset..offset + attribute.len()].copy_from_slice(attribute);
            offset += attribute.len();
        }
        bytes[offset..offset + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        bytes[24..28].copy_from_slice(&((offset + 4) as u32).to_le_bytes());
        bytes[42..44].copy_from_slice(&0xabcdu16.to_le_bytes());
        for (index, trailer) in [510, 1022].into_iter().enumerate() {
            let replacement = [bytes[trailer], bytes[trailer + 1]];
            bytes[44 + index * 2..46 + index * 2].copy_from_slice(&replacement);
            bytes[trailer..trailer + 2].copy_from_slice(&0xabcdu16.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn raw_builders_skip_malformed_data_before_valid_unnamed_stream() {
        let mut invalid_name = resident_attribute(0x80, b"wrong");
        invalid_name[9] = 1;
        invalid_name[10..12].copy_from_slice(&u16::MAX.to_le_bytes());
        let mut truncated_value = resident_attribute(0x80, b"wrong");
        truncated_value[16..20].copy_from_slice(&u32::MAX.to_le_bytes());
        let mut overlapping_value = resident_attribute(0x80, b"wrong");
        overlapping_value[20..22].copy_from_slice(&0u16.to_le_bytes());

        for malformed in [invalid_name, truncated_value, overlapping_value] {
            let mut bytes = record_bytes(&[malformed, resident_attribute(0x80, b"ok!")]);
            let record = FileRecord::parse(24, None, &mut bytes).unwrap();
            let (rich, _) =
                RawMftEntry::from_record_with_attr_list(&record, EntryBuildOptions::full());
            let (lean, _) = RawMftBatchScratch::from_record_with_attr_list(&record, true);
            assert_eq!(rich.real_size, 3);
            assert_eq!(rich.allocated_size, 3);
            assert!(rich.has_unnamed_data);
            assert!(rich.alternate_data_streams.is_empty());
            assert_eq!(lean.entry.real_size, 3);
            assert_eq!(lean.entry.allocated_size, 3);
        }
    }

    #[test]
    fn raw_builders_do_not_invent_metadata_from_malformed_attributes() {
        let mut invalid_name_value = vec![0; 68];
        invalid_name_value[64] = 1;
        invalid_name_value[65] = 4;
        invalid_name_value[66..68].copy_from_slice(&u16::from(b'x').to_le_bytes());
        let mut invalid_data = resident_attribute(0x80, b"wrong");
        invalid_data[16..20].copy_from_slice(&u32::MAX.to_le_bytes());
        let mut bytes =
            record_bytes(&[resident_attribute(0x30, &invalid_name_value), invalid_data]);
        let record = FileRecord::parse(24, None, &mut bytes).unwrap();
        let (rich, _) = RawMftEntry::from_record_with_attr_list(&record, EntryBuildOptions::full());
        let (lean, _) = RawMftBatchScratch::from_record_with_attr_list(&record, true);
        assert!(rich.file_name.is_empty());
        assert!(!rich.has_unnamed_data);
        assert_eq!(rich.metadata().file_names, crate::MftFieldState::Unknown);
        assert_eq!(rich.metadata().unnamed_data, crate::MftFieldState::Unknown);
        assert!(lean.entry.file_name.is_empty());
        assert!(!lean.have_unnamed_data);
    }

    #[test]
    fn raw_builders_do_not_take_sizes_from_data_continuations() {
        let mut continuation = vec![0; 72];
        continuation[..4].copy_from_slice(&0x80u32.to_le_bytes());
        continuation[4..8].copy_from_slice(&72u32.to_le_bytes());
        continuation[8] = 1;
        continuation[16..24].copy_from_slice(&1i64.to_le_bytes());
        continuation[24..32].copy_from_slice(&1i64.to_le_bytes());
        continuation[32..34].copy_from_slice(&64u16.to_le_bytes());
        continuation[40..48].copy_from_slice(&4096u64.to_le_bytes());
        continuation[48..56].copy_from_slice(&100u64.to_le_bytes());
        continuation[64..68].copy_from_slice(&[0x11, 1, 1, 0]);

        for include_initial in [false, true] {
            let mut attributes = vec![continuation.clone()];
            if include_initial {
                attributes.push(resident_attribute(0x80, b"ok!"));
            }
            let mut bytes = record_bytes(&attributes);
            let record = FileRecord::parse(24, None, &mut bytes).unwrap();
            let (rich, _) =
                RawMftEntry::from_record_with_attr_list(&record, EntryBuildOptions::full());
            let (lean, _) = RawMftBatchScratch::from_record_with_attr_list(&record, true);
            assert_eq!(rich.has_unnamed_data, include_initial);
            assert_eq!(lean.have_unnamed_data, include_initial);
            let expected_size = if include_initial { 3 } else { 0 };
            assert_eq!(rich.real_size, expected_size);
            assert_eq!(rich.allocated_size, expected_size);
            assert_eq!(lean.entry.real_size, expected_size);
            assert_eq!(lean.entry.allocated_size, expected_size);
            if !include_initial {
                assert_eq!(rich.metadata().unnamed_data, crate::MftFieldState::Unknown);
            }
        }
    }
}
