//! Shared FILE-record inspection for logical-MFT streams and volume scans.

use super::{
    MftDiagnostic, MftDiagnosticKind, MftExternalPayload, MftFieldState, MftMetadata,
    layout::{
        attribute::{AttributeListReference, NtfsAttribute, for_each_attribute_checked},
        data_run::validate_runs,
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

    pub fn into_diagnostics_and_external_payloads(
        self,
    ) -> (Vec<MftDiagnostic>, Vec<MftExternalPayload>) {
        (self.diagnostics, self.external_payloads)
    }

    /// Validate before dispatch so malformed named data cannot become unnamed data.
    pub fn observe(&mut self, number: u64, attr: &NtfsAttribute<'_>) -> bool {
        let kind = attr.type_id();
        if !attribute_shape_is_valid(attr) {
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
                    if validate_runs(&attr.data()[header.data_runs_offset as usize..]).is_err() {
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

/// Validate the in-record shape needed before any consumer interprets an
/// attribute. Expensive evidence checks such as runlist decoding remain in
/// `RecordInspection::observe` and are paid only by diagnostic callers.
pub(in crate::raw_mft) fn attribute_shape_is_valid(attr: &NtfsAttribute<'_>) -> bool {
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
        attr.nonresident_header().is_some_and(|header| {
            header.lowest_vcn >= 0
                && header.data_runs_offset as usize >= payload_min
                && (header.data_runs_offset as usize) < attr.len()
        })
    } else {
        attr.resident_header()
            .is_some_and(|header| header.value_offset as usize >= payload_min)
            && attr.resident_value().is_some()
    };
    let valid_typed = match attr.type_id() {
        0x10 => attr.as_standard_info().is_some(),
        0x30 => attr
            .as_file_name()
            .is_some_and(|(header, _)| header.namespace <= 3),
        0xc0 if !attr.is_non_resident() => {
            attr.resident_value().is_some_and(|value| value.len() >= 4)
        }
        _ => true,
    };
    attr.header.is_non_resident <= 1 && valid_name && valid_payload && valid_typed
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

/// Result of inspecting one FILE record against a set of exact
/// `$ATTRIBUTE_LIST` references.
pub(in crate::raw_mft) struct ReferencedAttributeInspection<'a> {
    /// Structurally valid attributes selected at most once from this record.
    pub(in crate::raw_mft) attributes: Vec<NtfsAttribute<'a>>,
    /// Attribute types whose reference did not bind to this record generation.
    pub(in crate::raw_mft) mismatches: Vec<u32>,
    /// Whether malformed attribute bytes may have hidden another referenced item.
    pub(in crate::raw_mft) malformed: bool,
    /// Whether an extension record itself contains an `$ATTRIBUTE_LIST`.
    pub(in crate::raw_mft) nested_attribute_list: bool,
}

/// Inspect one loaded record once, then bind exact list references to its
/// structurally valid attributes. Source generation/base validation and the
/// `(type, id, name, lowest VCN)` match therefore cannot diverge by caller.
pub(in crate::raw_mft) fn inspect_referenced_attributes<'a, 'r>(
    view: &FileRecord<'a>,
    references: impl IntoIterator<Item = &'r AttributeListReference>,
    base_record_number: u64,
    base_file_reference: u64,
) -> ReferencedAttributeInspection<'a> {
    let references = references.into_iter().collect::<Vec<_>>();
    if !references
        .iter()
        .any(|reference| reference.matches_record(view, base_record_number, base_file_reference))
    {
        return ReferencedAttributeInspection {
            attributes: Vec::new(),
            mismatches: references
                .into_iter()
                .map(|reference| reference.type_id)
                .collect(),
            malformed: false,
            nested_attribute_list: false,
        };
    }
    let (candidates, malformed) = inspect_attribute_shapes(view);
    select_referenced_attributes(
        view,
        &candidates,
        references,
        base_record_number,
        base_file_reference,
        malformed,
    )
}

/// Inspect only attribute framing and typed in-record payload boundaries.
/// This is the selection hot path for live enrichment and bootstrap discovery.
fn inspect_attribute_shapes<'a>(view: &FileRecord<'a>) -> (Vec<NtfsAttribute<'a>>, bool) {
    let (offset, used) = view.attrs_range();
    let mut attributes = Vec::new();
    let mut malformed = false;
    let complete = for_each_attribute_checked(view.data, offset, used, |attribute| {
        if attribute_shape_is_valid(attribute) {
            if let Some(attribute) = NtfsAttribute::new(attribute.data()) {
                attributes.push(attribute);
            }
        } else {
            malformed = true;
        }
    });
    (attributes, malformed || !complete)
}

/// Bind references against an attribute set that the caller already inspected.
/// This avoids rescanning records when diagnostics and selection are both needed.
pub(in crate::raw_mft) fn select_referenced_attributes<'a, 'r>(
    view: &FileRecord<'a>,
    candidates: &[NtfsAttribute<'a>],
    references: impl IntoIterator<Item = &'r AttributeListReference>,
    base_record_number: u64,
    base_file_reference: u64,
    malformed: bool,
) -> ReferencedAttributeInspection<'a> {
    let nested_attribute_list = view.number != base_record_number
        && candidates
            .iter()
            .any(|attribute| attribute.type_id() == 0x20);
    let mut selected_ids = std::collections::BTreeSet::new();
    let mut attributes = Vec::new();
    let mut mismatches = Vec::new();

    for reference in references {
        if !reference.matches_record(view, base_record_number, base_file_reference) {
            mismatches.push(reference.type_id);
            continue;
        }
        match candidates
            .iter()
            .find(|attribute| reference.matches(attribute))
        {
            Some(attribute) if selected_ids.insert(attribute.header.id) => {
                if let Some(attribute) = NtfsAttribute::new(attribute.data()) {
                    attributes.push(attribute);
                }
            }
            Some(_) => {}
            None => mismatches.push(reference.type_id),
        }
    }

    ReferencedAttributeInspection {
        attributes,
        mismatches,
        malformed,
        nested_attribute_list,
    }
}
