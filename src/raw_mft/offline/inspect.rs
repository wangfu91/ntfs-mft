//! Offline diagnostics around the shared, intentionally tolerant entry builder.

use super::*;
use crate::raw_mft::layout::{
    attribute::{AttributeListEntryHeader, for_each_attribute_checked},
    data_run::summarize_runs,
    record::FileRecordHeader,
};
use zerocopy::FromBytes;

pub(super) struct RecordData {
    pub number: u64,
    pub bytes: Vec<u8>,
    pub header: FileRecordHeader,
}

impl RecordData {
    pub fn view(&self) -> FileRecord<'_> {
        FileRecord {
            data: &self.bytes,
            header: self.header,
            number: self.number,
        }
    }
}

#[derive(Default)]
pub(in crate::raw_mft) struct Inspection {
    present: [bool; 5],
    incomplete: [bool; 5],
    pub diagnostics: Vec<MftDiagnostic>,
    pub external_payloads: Vec<MftExternalPayload>,
}

impl Inspection {
    pub fn issue(&mut self, number: u64, attribute_type: Option<u32>, issue: MftIssue) {
        match attribute_type {
            Some(0x10) => self.incomplete[0] = true,
            Some(0x30) => self.incomplete[1] = true,
            Some(0x80) => {
                self.incomplete[2] = true;
                self.incomplete[3] = true;
            }
            Some(0xc0) => self.incomplete[4] = true,
            _ => self.incomplete.fill(true),
        }
        self.diagnostics.push(MftDiagnostic {
            record_number: number,
            attribute_type,
            issue,
        });
    }

    pub fn metadata(&self) -> MftMetadata {
        let state = |i| match (self.present[i], self.incomplete[i]) {
            (true, incomplete) => MftFieldState::Present {
                complete: !incomplete,
            },
            (false, false) => MftFieldState::Absent,
            (false, true) => MftFieldState::Unknown,
        };
        MftMetadata {
            standard_information: state(0),
            file_names: state(1),
            unnamed_data: state(2),
            alternate_data_streams: state(3),
            reparse_point: state(4),
        }
    }

    /// Validate before dispatch so malformed named data cannot become unnamed data.
    pub fn observe(&mut self, number: u64, attr: &NtfsAttribute<'_>) -> bool {
        let kind = attr.type_id();
        let valid_name = !attr.has_name() || attr.name_bytes().is_some();
        let header_size = if attr.is_non_resident() { 64 } else { 24 };
        let valid_name =
            valid_name && (!attr.has_name() || attr.header.name_offset as usize >= header_size);
        let payload_min = if attr.has_name() {
            header_size.max(attr.header.name_offset as usize + attr.header.name_length as usize * 2)
        } else {
            header_size
        };
        let valid_payload = if attr.is_non_resident() {
            attr.nonresident_header().is_some_and(|h| {
                h.lowest_vcn >= 0
                    && h.data_runs_offset as usize >= payload_min
                    && (h.data_runs_offset as usize) < attr.len()
            })
        } else {
            attr.resident_header()
                .is_some_and(|h| h.value_offset as usize >= payload_min)
                && attr.resident_value().is_some()
        };
        let valid_typed = match kind {
            0x10 => attr.as_standard_info().is_some(),
            0x30 => attr.as_file_name().is_some_and(|(h, _)| h.namespace <= 3),
            0xc0 if !attr.is_non_resident() => attr.resident_value().is_some_and(|v| v.len() >= 4),
            _ => true,
        };
        if attr.header.is_non_resident > 1 || !valid_name || !valid_payload || !valid_typed {
            self.issue(number, Some(kind), MftIssue::MalformedAttribute);
            return false;
        }
        if attr.is_non_resident() {
            self.external_payloads.push(MftExternalPayload {
                record_number: number,
                attribute_type: kind,
                attribute_id: attr.header.id,
            });
        }
        match kind {
            0x10 => self.present[0] = true,
            0x30 => self.present[1] = true,
            0x80 => {
                self.present[if attr.has_name() { 3 } else { 2 }] = true;
                if let Some(header) = attr.nonresident_header() {
                    // Stream sizes are authoritative only in the initial segment.
                    if header.lowest_vcn == 0 && header.initialized_size > header.data_size {
                        self.issue(number, Some(kind), MftIssue::MalformedAttribute);
                    }
                    if header.lowest_vcn != 0 {
                        self.issue(number, Some(kind), MftIssue::SplitDataStream);
                        // The existing builder uses the first segment's sizes; do not
                        // replace them with continuation-header placeholder values.
                    }
                    if summarize_runs(&attr.data()[header.data_runs_offset as usize..]).is_err() {
                        self.issue(number, Some(kind), MftIssue::MalformedAttribute);
                    }
                }
            }
            0xc0 => {
                if attr.is_non_resident() {
                    self.issue(number, Some(kind), MftIssue::ExternalPayloadUnavailable);
                } else {
                    self.present[4] = true;
                }
            }
            _ => {}
        }
        true
    }
}

pub(super) fn inspect_record<'a>(
    record: &'a RecordData,
    inspection: &mut Inspection,
) -> Vec<NtfsAttribute<'a>> {
    inspect_view(&record.view(), inspection)
}

pub(in crate::raw_mft) fn inspect_view<'a>(
    view: &FileRecord<'a>,
    inspection: &mut Inspection,
) -> Vec<NtfsAttribute<'a>> {
    let (offset, used) = view.attrs_range();
    let mut attributes = Vec::new();
    let complete = for_each_attribute_checked(view.data, offset, used, |attr| {
        if inspection.observe(view.number, attr) {
            // NtfsAttribute deliberately does not expose Clone. Reborrow the
            // already-validated exact slice without copying any payload bytes.
            if let Some(attr) = NtfsAttribute::new(attr.data()) {
                attributes.push(attr);
            }
        }
    });
    if !complete {
        inspection.issue(view.number, None, MftIssue::MalformedAttribute);
    }
    attributes
}

pub(super) struct ListReference {
    pub type_id: u32,
    pub file_reference: u64,
    id: u16,
    lowest_vcn: i64,
    name: Vec<u8>,
}

impl ListReference {
    pub fn matches(&self, attr: &NtfsAttribute<'_>) -> bool {
        self.type_id == attr.type_id()
            && self.id == attr.header.id
            && self.name.as_slice() == attr.name_bytes().unwrap_or_default()
            && self.lowest_vcn == attr.nonresident_header().map_or(0, |h| h.lowest_vcn)
    }
}

pub(super) fn list_references(
    attr: &NtfsAttribute<'_>,
    number: u64,
    inspection: &mut Inspection,
) -> Vec<ListReference> {
    let Some(data) = attr.resident_value() else {
        return Vec::new();
    };
    let mut result = Vec::new();
    let mut offset = 0;
    while offset < data.len() {
        let remaining = &data[offset..];
        // Some lists include zero-filled alignment padding after their entries.
        if remaining.len() < 8 && remaining.iter().all(|b| *b == 0) {
            break;
        }
        let header_size = std::mem::size_of::<AttributeListEntryHeader>();
        let header = remaining
            .get(..header_size)
            .and_then(|bytes| AttributeListEntryHeader::read_from_bytes(bytes).ok());
        let Some(h) = header else {
            inspection.issue(number, None, MftIssue::MalformedAttributeList);
            break;
        };
        let len = h.record_length as usize;
        let name_start = h.attribute_name_offset as usize;
        let name_len = h.attribute_name_length as usize * 2;
        if len < header_size
            || len > remaining.len()
            || h.lowest_vcn < 0
            || (name_len != 0 && (name_start < header_size || name_start + name_len > len))
        {
            inspection.issue(number, None, MftIssue::MalformedAttributeList);
            break;
        }
        result.push(ListReference {
            type_id: h.type_id,
            file_reference: h.file_reference,
            id: h.attribute_id,
            lowest_vcn: h.lowest_vcn,
            name: if name_len == 0 {
                Vec::new()
            } else {
                remaining[name_start..name_start + name_len].to_vec()
            },
        });
        offset += len;
    }
    result
}
