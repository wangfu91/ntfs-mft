//! Shared metadata evidence for volume scans and logical stream parsing.

/// Whether metadata was observed and whether its supported attributes were inspected.
///
/// This describes metadata, never the availability of arbitrary file contents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MftFieldState {
    /// Collection was explicitly disabled for this metadata category.
    NotCollected,
    /// At least one valid attribute was observed. `complete` is false when more
    /// metadata may be missing or a split stream could not be fully summarized.
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
    /// Unnamed `$DATA` metadata, including its run summary.
    pub unnamed_data: MftFieldState,
    /// Named `$DATA` stream metadata.
    pub alternate_data_streams: MftFieldState,
    /// `$REPARSE_POINT` tag metadata (not a tag copied from `$FILE_NAME`).
    pub reparse_point: MftFieldState,
}

