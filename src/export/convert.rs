use super::*;
use crate::{
    Filetime, MftError, MftFieldState, MftIssue, MftStreamRecord, NtfsFileReference, RawMftEntry,
};
use chrono::{Datelike, TimeZone, Utc};
use std::ffi::OsStr;

impl ExportName {
    /// Preserve original NTFS code units on every host platform.
    pub fn from_ntfs_name(name: &crate::NtfsName) -> Self {
        Self {
            display: name.to_string_lossy(),
            raw: NameUnits::WindowsUtf16(name.as_utf16().to_vec()),
        }
    }

    /// Preserve the supplied OS string without treating its display text as authoritative.
    pub fn from_os_str(name: &OsStr) -> Self {
        #[cfg(windows)]
        let raw = {
            use std::os::windows::ffi::OsStrExt;
            NameUnits::WindowsUtf16(name.encode_wide().collect())
        };
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        let raw = {
            use std::os::unix::ffi::OsStrExt;
            NameUnits::UnixBytes(name.as_bytes().to_vec())
        };
        Self {
            display: name.to_string_lossy().into_owned(),
            raw,
        }
    }
}

impl From<NtfsFileReference> for ExportReference {
    fn from(reference: NtfsFileReference) -> Self {
        Self {
            raw: reference.get().to_string(),
            record_number: reference.record_number().to_string(),
            sequence_number: reference.sequence(),
        }
    }
}

impl From<Filetime> for ExportTimestamp {
    fn from(value: Filetime) -> Self {
        let nanos = value.to_unix_nanos();
        // Euclidean division preserves fractions before the Unix epoch.
        let secs = nanos.div_euclid(1_000_000_000) as i64;
        let fraction = nanos.rem_euclid(1_000_000_000) as u32;
        let utc = Utc
            .timestamp_opt(secs, fraction)
            .single()
            .filter(|date| (0..=9999).contains(&date.year()))
            .map(|date| {
                format!(
                    "{}.{:07}Z",
                    date.format("%Y-%m-%dT%H:%M:%S"),
                    fraction / 100
                )
            });
        let status = if value.is_zero() {
            TimestampStatus::ZeroAmbiguous
        } else if utc.is_some() {
            TimestampStatus::Formatted
        } else {
            TimestampStatus::OutOfRange
        };
        Self {
            ticks: value.raw().to_string(),
            utc,
            status,
        }
    }
}

impl From<MftFieldState> for FieldState {
    fn from(value: MftFieldState) -> Self {
        match value {
            MftFieldState::NotCollected => Self::NotCollected,
            MftFieldState::Present { complete: true } => Self::Present,
            MftFieldState::Present { complete: false } => Self::Partial,
            MftFieldState::Absent => Self::Absent,
            MftFieldState::Unknown => Self::Unknown,
        }
    }
}

impl From<&MftError> for ExportError {
    fn from(error: &MftError) -> Self {
        let (record_number, volume_offset, reported_offset) = match error {
            MftError::InvalidMftRecordAt {
                number,
                volume_offset,
                ..
            } => (
                Some(number.to_string()),
                Some(volume_offset.to_string()),
                None,
            ),
            MftError::InvalidMftRecord { number, .. } | MftError::FixupMismatch { number } => {
                (Some(number.to_string()), None, None)
            }
            MftError::TruncatedRecord { offset, .. }
            | MftError::MisalignedRecord { offset, .. }
            | MftError::InvalidRecord { offset, .. } => (None, None, Some(offset.to_string())),
            _ => (None, None, None),
        };
        Self {
            category: if error.is_io_error() {
                "io"
            } else if error.is_parse_error() {
                "parse"
            } else {
                "other"
            }
            .into(),
            message: error.to_string(),
            record_number,
            volume_offset,
            reported_offset,
        }
    }
}

fn observed(collected: Option<bool>, nonempty: bool) -> FieldState {
    if collected == Some(false) {
        FieldState::NotCollected
    } else if nonempty {
        FieldState::Partial
    } else if collected == Some(true) {
        FieldState::NotObserved
    } else {
        FieldState::Unknown
    }
}

impl ExportRecordV1 {
    /// Export an existing scan result. This cannot recover discarded parser evidence.
    pub fn from_entry(entry: &RawMftEntry) -> Self {
        let options = entry.collection_options;
        Self {
            source: "entry".into(),
            record_number: entry.record_number.to_string(),
            sequence_number: entry.sequence_number,
            reference: entry.file_reference.into(),
            base_record_number: entry.base_record_number.to_string(),
            base_sequence_number: None,
            stream_offset: None,
            hard_link_count: entry.hard_link_count,
            flags: entry.flags,
            used: entry.is_used,
            directory: entry.is_directory,
            reparse_point: entry.is_reparse_point,
            reparse_tag: entry.reparse_tag,
            selected_name: ExportLink {
                parent: entry.parent_reference.into(),
                namespace: entry.namespace as u8,
                name: ExportName::from_ntfs_name(&entry.file_name),
            },
            links: entry
                .links
                .iter()
                .map(|link| ExportLink {
                    parent: link.parent_reference.into(),
                    namespace: link.namespace as u8,
                    name: ExportName::from_ntfs_name(&link.file_name),
                })
                .collect(),
            standard_information_times: ExportTimes {
                created: entry.si_created.into(),
                modified: entry.si_modified.into(),
                mft_modified: entry.si_mft_modified.into(),
                accessed: entry.si_accessed.into(),
            },
            file_name_times: ExportTimes {
                created: entry.fn_created.into(),
                modified: entry.fn_modified.into(),
                mft_modified: entry.fn_mft_modified.into(),
                accessed: entry.fn_accessed.into(),
            },
            file_attributes: entry.si_file_attributes.bits(),
            unnamed_stream: entry.has_unnamed_data.then(|| ExportStream {
                name: None,
                real_size: entry.real_size.to_string(),
                allocated_size: entry.allocated_size.to_string(),
                resident: entry.is_resident,
            }),
            sparse: entry.is_sparse,
            compressed: entry.is_compressed,
            encrypted: entry.is_encrypted,
            alternate_streams: entry
                .alternate_data_streams
                .iter()
                .map(|stream| ExportStream {
                    name: Some(ExportName::from_ntfs_name(&stream.name)),
                    real_size: stream.real_size.to_string(),
                    allocated_size: stream.allocated_size.to_string(),
                    resident: stream.is_resident,
                })
                .collect(),
            metadata: ExportMetadata {
                standard_information: FieldState::Unknown,
                file_names: observed(None, !entry.file_name.is_empty() || !entry.links.is_empty()),
                unnamed_data: observed(Some(true), entry.has_unnamed_data),
                alternate_data_streams: observed(
                    Some(options.collect_alternate_data_streams()),
                    !entry.alternate_data_streams.is_empty(),
                ),
                reparse_point: FieldState::Unknown,
                dos_links_collected: Some(options.collect_dos_file_name_links()),
            },
            path: ExportPath::NotRequested,
            diagnostics: Vec::new(),
            external_payloads: Vec::new(),
            name_fidelity: "ntfs_utf16_preserved".into(),
        }
    }

    /// Preserve the logical-stream parser's explicit evidence and diagnostics.
    pub fn from_stream(record: &MftStreamRecord) -> Self {
        let mut dto = Self::from_entry(&record.entry);
        dto.source = "mft_stream".into();
        dto.stream_offset = Some(record.stream_offset.to_string());
        dto.apply_inspection(record.metadata, &record.diagnostics);
        dto.external_payloads = record
            .external_payloads
            .iter()
            .map(|p| ExportExternalPayload {
                record_number: p.record_number.to_string(),
                attribute_type: p.attribute_type,
                attribute_id: p.attribute_id,
            })
            .collect();
        dto
    }
    /// Preserve raw diagnostic metadata and collection policy without claiming
    /// unverified extensions are complete.
    pub fn from_diagnostic(record: &crate::RawMftDiagnosticRecord) -> Self {
        let options = record.entry.collection_options;
        let mut dto = Self::from_entry(&record.entry);
        dto.source = "volume_scan".into();
        dto.apply_inspection(record.metadata, &record.diagnostics);
        dto.metadata.dos_links_collected = Some(options.collect_dos_file_name_links());
        dto
    }
    fn apply_inspection(
        &mut self,
        metadata: crate::MftMetadata,
        diagnostics: &[crate::MftDiagnostic],
    ) {
        let m = metadata;
        self.metadata = ExportMetadata {
            standard_information: m.standard_information.into(),
            file_names: m.file_names.into(),
            unnamed_data: m.unnamed_data.into(),
            alternate_data_streams: m.alternate_data_streams.into(),
            reparse_point: m.reparse_point.into(),
            dos_links_collected: Some(true),
        };
        self.diagnostics = diagnostics
            .iter()
            .map(|d| {
                let (code, reference, error) = match &d.issue {
                    MftIssue::EnrichmentNotVerified => ("enrichment_not_verified", None, None),
                    MftIssue::MalformedAttribute => ("malformed_attribute", None, None),
                    MftIssue::ExtensionsNotResolved => ("extensions_not_resolved", None, None),
                    MftIssue::ExternalPayloadUnavailable => {
                        ("external_payload_unavailable", None, None)
                    }
                    MftIssue::MalformedAttributeList => ("malformed_attribute_list", None, None),
                    MftIssue::ExtensionRead { reference, error } => (
                        "extension_read",
                        Some(NtfsFileReference::new(*reference).into()),
                        Some(error.into()),
                    ),
                    MftIssue::ExtensionMismatch => ("extension_mismatch", None, None),
                    MftIssue::NestedAttributeList => ("nested_attribute_list", None, None),
                    MftIssue::SplitDataStream => ("split_data_stream", None, None),
                    MftIssue::ExtensionRecordOnly => ("extension_record_only", None, None),
                };
                ExportDiagnostic {
                    record_number: d.record_number.to_string(),
                    attribute_type: d.attribute_type,
                    code: code.into(),
                    reference,
                    error,
                }
            })
            .collect();
    }
}
