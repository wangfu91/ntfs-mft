//! Shared metadata evidence for volume scans and logical stream parsing.
use crate::MftError;

/// Whether metadata was observed and whether its supported attributes were inspected.
///
/// This describes metadata, never the availability of arbitrary file contents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MftFieldState {
    /// Collection was explicitly disabled for this metadata category.
    NotCollected,
    /// At least one valid attribute was observed. `complete` is false when more
    /// metadata may be missing or a split stream could not be fully inspected.
    Present {
        /// All relevant supported metadata was inspected successfully.
        complete: bool,
    },
    /// No such attribute exists in the fully inspected record set.
    Absent,
    /// No valid attribute was observed, but inspection was incomplete.
    Unknown,
}

/// Completeness of the metadata categories represented by [`super::RawMftEntry`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MftMetadata {
    /// `$STANDARD_INFORMATION` timestamps and flags.
    pub standard_information: MftFieldState,
    /// `$FILE_NAME` names, links, and timestamps.
    pub file_names: MftFieldState,
    /// Unnamed `$DATA` metadata.
    pub unnamed_data: MftFieldState,
    /// Named `$DATA` stream metadata.
    pub alternate_data_streams: MftFieldState,
    /// `$REPARSE_POINT` tag metadata (not a tag copied from `$FILE_NAME`).
    pub reparse_point: MftFieldState,
}

/// An attribute whose payload is outside the logical MFT stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MftExternalPayload {
    /// Physical MFT record containing the attribute header.
    pub record_number: u64,
    /// NTFS attribute type code.
    pub attribute_type: u32,
    /// Attribute instance identifier within that record.
    pub attribute_id: u16,
}

/// Structured category explaining why supported MFT metadata could not be
/// inspected completely.
#[derive(Debug)]
#[non_exhaustive]
pub enum MftDiagnosticKind {
    /// Raw scanning enriched selected fields but did not validate the complete
    /// extension attribute set. Presence may still be observed in the base.
    EnrichmentNotVerified,
    /// Attribute framing or a typed payload was malformed.
    MalformedAttribute,
    /// Extension lookup was disabled by the caller.
    ExtensionsNotResolved,
    /// A non-resident payload requires the original volume.
    ExternalPayloadUnavailable,
    /// A resident attribute list was malformed.
    MalformedAttributeList,
    /// A referenced record could not be read or validated.
    ExtensionRead {
        /// Full reference, including sequence number.
        reference: u64,
        /// Original item-level error.
        error: MftError,
    },
    /// The reference sequence, base reference, or selected attribute did not match.
    ExtensionMismatch,
    /// An extension has its own attribute list; recursive traversal is unsupported.
    NestedAttributeList,
    /// A data stream spans attribute segments; its metadata may be incomplete.
    SplitDataStream,
    /// A physical extension record was requested without resolving its base.
    ExtensionRecordOnly,
}

/// A recoverable diagnostic associated with a parsed record.
#[derive(Debug)]
pub struct MftDiagnostic {
    /// Record at which the diagnostic was observed.
    pub record_number: u64,
    /// Affected attribute type, or `None` when all categories may be affected.
    pub attribute_type: Option<u32>,
    /// Structured diagnostic category, including underlying errors where applicable.
    pub kind: MftDiagnosticKind,
}
