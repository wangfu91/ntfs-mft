//! Shared FILE-record inspection for logical-MFT streams and volume scans.

use super::{
    MftDiagnostic, MftDiagnosticKind, MftExternalPayload, MftFieldState, MftMetadata,
    layout::{
        attribute::{NtfsAttribute, for_each_attribute_checked},
        data_run::decode_runs,
        record::FileRecord,
    },
};

#[derive(Default)]
struct Evidence {
    observed: bool,
    incomplete: bool,
}

impl Evidence {
    fn state(&self) -> MftFieldState {
        match (self.observed, self.incomplete) {
            (true, incomplete) => MftFieldState::Present {
                complete: !incomplete,
            },
            (false, false) => MftFieldState::Absent,
            (false, true) => MftFieldState::Unknown,
        }
    }
}

#[derive(Default)]
struct FieldEvidence {
    standard_information: Evidence,
    file_names: Evidence,
    unnamed_data: Evidence,
    alternate_data_streams: Evidence,
    reparse_point: Evidence,
}

impl FieldEvidence {
    fn mark_incomplete(&mut self, attribute_type: Option<u32>) {
        match attribute_type {
            Some(0x10) => self.standard_information.incomplete = true,
            Some(0x30) => self.file_names.incomplete = true,
            Some(0x80) => {
                self.unnamed_data.incomplete = true;
                self.alternate_data_streams.incomplete = true;
            }
            Some(0xc0) => self.reparse_point.incomplete = true,
            _ => {
                self.standard_information.incomplete = true;
                self.file_names.incomplete = true;
                self.unnamed_data.incomplete = true;
                self.alternate_data_streams.incomplete = true;
                self.reparse_point.incomplete = true;
            }
        }
    }

    fn metadata(&self) -> MftMetadata {
        MftMetadata {
            standard_information: self.standard_information.state(),
            file_names: self.file_names.state(),
            unnamed_data: self.unnamed_data.state(),
            alternate_data_streams: self.alternate_data_streams.state(),
            reparse_point: self.reparse_point.state(),
        }
    }
}

/// Accumulates validation evidence and recoverable limitations for one FILE record.
#[derive(Default)]
pub(in crate::raw_mft) struct RecordInspection {
    fields: FieldEvidence,
    diagnostics: Vec<MftDiagnostic>,
    external_payloads: Vec<MftExternalPayload>,
}

impl RecordInspection {
    pub fn diagnose(&mut self, number: u64, attribute_type: Option<u32>, kind: MftDiagnosticKind) {
        self.fields.mark_incomplete(attribute_type);
        self.diagnostics.push(MftDiagnostic {
            record_number: number,
            attribute_type,
            kind,
        });
    }

    pub fn metadata(&self) -> MftMetadata {
        self.fields.metadata()
    }

    pub fn has_malformed_attribute(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|diagnostic| matches!(diagnostic.kind, MftDiagnosticKind::MalformedAttribute))
    }

    pub fn into_diagnostics(self) -> Vec<MftDiagnostic> {
        self.diagnostics
    }

    pub fn into_diagnostics_and_external_payloads(
        self,
    ) -> (Vec<MftDiagnostic>, Vec<MftExternalPayload>) {
        (self.diagnostics, self.external_payloads)
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
            self.diagnose(number, Some(kind), MftDiagnosticKind::MalformedAttribute);
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
            0x10 => self.fields.standard_information.observed = true,
            0x30 => self.fields.file_names.observed = true,
            0x80 => {
                if attr.has_name() {
                    self.fields.alternate_data_streams.observed = true;
                } else {
                    self.fields.unnamed_data.observed = true;
                }
                if let Some(header) = attr.nonresident_header() {
                    // Stream sizes are authoritative only in the initial segment.
                    if header.lowest_vcn == 0 && header.initialized_size > header.data_size {
                        self.diagnose(number, Some(kind), MftDiagnosticKind::MalformedAttribute);
                    }
                    if header.lowest_vcn != 0 {
                        self.diagnose(number, Some(kind), MftDiagnosticKind::SplitDataStream);
                        // The existing builder uses the first segment's sizes; do not
                        // replace them with continuation-header placeholder values.
                    }
                    if decode_runs(&attr.data()[header.data_runs_offset as usize..]).is_err() {
                        self.diagnose(number, Some(kind), MftDiagnosticKind::MalformedAttribute);
                    }
                }
            }
            0xc0 => {
                if attr.is_non_resident() {
                    self.diagnose(
                        number,
                        Some(kind),
                        MftDiagnosticKind::ExternalPayloadUnavailable,
                    );
                } else {
                    self.fields.reparse_point.observed = true;
                }
            }
            _ => {}
        }
        true
    }
}

pub(in crate::raw_mft) fn inspect_view<'a>(
    view: &FileRecord<'a>,
    inspection: &mut RecordInspection,
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
        inspection.diagnose(view.number, None, MftDiagnosticKind::MalformedAttribute);
    }
    attributes
}
