use super::*;
use crate::{
    Fid, Filetime, MftError, OfflineFieldState, OfflineIssue, OfflineMftRecord, RawMftEntry,
};
use chrono::{Datelike, TimeZone, Utc};
use std::ffi::OsStr;

impl ExportName {
    /// Preserve the supplied OS string without treating its display text as authoritative.
    pub fn from_os_str(name: &OsStr) -> Self {
        #[cfg(windows)]
        let raw = {
            use std::os::windows::ffi::OsStrExt;
            NameUnits::WindowsUtf16(name.encode_wide().collect())
        };
        #[cfg(target_os = "linux")]
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

impl From<Fid> for ExportReference {
    fn from(fid: Fid) -> Self {
        Self {
            bits: if fid.is_standard() { 64 } else { 128 },
            raw: fid.as_u128().to_string(),
            record_number: fid.record_number().map(|v| v.to_string()),
            sequence_number: fid.sequence(),
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

impl From<OfflineFieldState> for FieldState {
    fn from(value: OfflineFieldState) -> Self {
        match value {
            OfflineFieldState::Present { complete: true } => Self::Present,
            OfflineFieldState::Present { complete: false } => Self::Partial,
            OfflineFieldState::Absent => Self::Absent,
            OfflineFieldState::Unknown => Self::Unknown,
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
    pub fn from_entry(entry: &RawMftEntry, context: &ExportContext) -> Self {
        let options = context.entry_options;
        Self {
            source: "raw_scan".into(),
            record_number: entry.record_number.to_string(),
            sequence_number: entry.sequence_number,
            reference: entry.file_reference.into(),
            base_record_number: entry.base_record_reference.to_string(),
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
                name: ExportName::from_os_str(&entry.file_name),
            },
            links: entry
                .links
                .iter()
                .map(|link| ExportLink {
                    parent: link.parent_reference.into(),
                    namespace: link.namespace as u8,
                    name: ExportName::from_os_str(&link.file_name),
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
            data_runs: entry.data_run_summary.as_ref().map(|runs| ExportRuns {
                run_count: runs.run_count,
                total_clusters: runs.total_clusters.to_string(),
            }),
            alternate_streams: entry
                .alternate_data_streams
                .iter()
                .map(|stream| ExportStream {
                    name: Some(ExportName::from_os_str(&stream.name)),
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
                    options.map(|o| o.collect_alternate_data_streams()),
                    !entry.alternate_data_streams.is_empty(),
                ),
                reparse_point: FieldState::Unknown,
                data_run_summary: observed(
                    options.map(|o| o.collect_data_run_summary()),
                    entry.data_run_summary.is_some(),
                ),
                dos_links_collected: options.map(|o| o.collect_dos_file_name_links()),
            },
            path: ExportPath::NotRequested,
            diagnostics: Vec::new(),
            external_payloads: Vec::new(),
            name_fidelity: if cfg!(windows) {
                "windows_utf16_preserved"
            } else {
                "original_ntfs_utf16_unavailable"
            }
            .into(),
        }
    }

    /// Preserve the merged offline API's explicit evidence and diagnostic structure.
    pub fn from_offline(record: &OfflineMftRecord) -> Self {
        let mut dto = Self::from_entry(&record.entry, &ExportContext::default());
        dto.source = "offline_stream".into();
        dto.stream_offset = Some(record.stream_offset.to_string());
        let m = record.metadata;
        dto.metadata = ExportMetadata {
            standard_information: m.standard_information.into(),
            file_names: m.file_names.into(),
            unnamed_data: m.unnamed_data.into(),
            alternate_data_streams: m.alternate_data_streams.into(),
            reparse_point: m.reparse_point.into(),
            data_run_summary: if dto.data_runs.is_some() {
                m.unnamed_data.into()
            } else if m.unnamed_data == OfflineFieldState::Absent {
                FieldState::Absent
            } else {
                FieldState::NotObserved
            },
            dos_links_collected: Some(true),
        };
        dto.diagnostics = record
            .diagnostics
            .iter()
            .map(|d| {
                let (code, reference, error) = match &d.issue {
                    OfflineIssue::MalformedAttribute => ("malformed_attribute", None, None),
                    OfflineIssue::ExtensionsNotResolved => ("extensions_not_resolved", None, None),
                    OfflineIssue::ExternalPayloadUnavailable => {
                        ("external_payload_unavailable", None, None)
                    }
                    OfflineIssue::MalformedAttributeList => {
                        ("malformed_attribute_list", None, None)
                    }
                    OfflineIssue::ExtensionRead { reference, error } => (
                        "extension_read",
                        Some(Fid::new(*reference).into()),
                        Some(error.into()),
                    ),
                    OfflineIssue::ExtensionMismatch => ("extension_mismatch", None, None),
                    OfflineIssue::NestedAttributeList => ("nested_attribute_list", None, None),
                    OfflineIssue::SplitDataStream => ("split_data_stream", None, None),
                    OfflineIssue::ExtensionRecordOnly => ("extension_record_only", None, None),
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
}
