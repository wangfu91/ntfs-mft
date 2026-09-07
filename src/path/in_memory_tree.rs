//! Volume-independent path indexing with explicit identity and quality checks.
use crate::{NtfsFileReference, NtfsName, RawMftBatchEntry, RawMftEntry};
use rustc_hash::{FxHashMap, FxHashSet};
use std::path::{Component, Path, PathBuf};

/// Why a path could not be reconstructed exactly.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum MftPathError {
    /// A required record was not indexed.
    #[error("missing indexed reference {0}")]
    MissingReference(NtfsFileReference),
    /// A slot contains a different generation of a file.
    #[error("reference {requested} was replaced by {indexed}")]
    SequenceMismatch {
        /// Reference requested by the record or caller.
        requested: NtfsFileReference,
        /// Reference currently indexed at that slot.
        indexed: NtfsFileReference,
    },
    /// Parent links form a cycle.
    #[error("cycle at reference {0}")]
    Cycle(NtfsFileReference),
    /// The parent walk exceeded its defensive bound.
    #[error("path exceeds 256 parent steps")]
    DepthLimit,
    /// A leaf name would be interpreted as a path instead of one component.
    #[error("invalid path component")]
    InvalidName,
}

/// Trust level of a best-effort indexed path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MftPathQuality {
    /// All references, including the root, matched their indexed sequence numbers.
    Exact,
    /// A complete chain required at least one record-number-only match.
    RecordFallback,
    /// The chain could not reach the indexed root.
    Partial,
}

/// An indexed relative path and evidence about its reconstruction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MftPathResolution {
    /// Observed path components in root-to-leaf order. For partial results this
    /// is only a suffix, not a path relative to the volume root.
    pub path: PathBuf,
    /// Whether the chain is exact, fell back to a reused slot, or is incomplete.
    pub quality: MftPathQuality,
    /// Whether any step ignored a mismatched sequence, including partial walks.
    pub used_record_fallback: bool,
    /// Reason the walk stopped, when incomplete.
    pub error: Option<MftPathError>,
}

#[derive(Debug, Clone)]
struct Node {
    reference: NtfsFileReference,
    parent: NtfsFileReference,
    name: NtfsName,
}

/// Reusable path index built from caller-supplied rich, lean or offline records.
///
/// Insert the root (record 5) and all needed parent records. Paths are relative
/// to that root, and use host path separators. This index performs no I/O and
/// does not create an atomic snapshot. Replacing a slot replaces its previous
/// generation. Memory is proportional to indexed names and records.
#[derive(Debug, Clone, Default)]
pub struct MftPathIndex {
    entries: FxHashMap<u64, Node>,
}

impl MftPathIndex {
    /// Create an empty index without scanning a volume.
    pub fn new() -> Self {
        Self::default()
    }
    /// Number of indexed records.
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    /// Whether the index has no records.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    /// Insert a rich record, including `MftStreamRecord::entry`. Nameless
    /// records are ignored. Caller decides whether unused records belong here.
    pub fn insert(&mut self, entry: &RawMftEntry) -> Result<bool, MftPathError> {
        self.insert_reference(
            entry.file_reference,
            entry.parent_reference,
            &entry.file_name,
        )
    }
    /// Insert a lean record during an existing parallel scan.
    pub fn insert_lean(&mut self, entry: &RawMftBatchEntry) -> Result<bool, MftPathError> {
        self.insert_reference(
            entry.file_reference,
            entry.parent_reference,
            &entry.file_name,
        )
    }
    /// Insert a named record. Returns false for an empty name. The supplied name
    /// must be one normal path component, except the root's conventional `.`.
    pub fn insert_reference(
        &mut self,
        reference: NtfsFileReference,
        parent: NtfsFileReference,
        name: &NtfsName,
    ) -> Result<bool, MftPathError> {
        let number = reference.record_number();
        if name.is_empty() {
            return Ok(false);
        }
        let native = name.to_os_string();
        let mut components = Path::new(&native).components();
        if !(number == 5 && name == &NtfsName::from("."))
            && !(matches!(components.next(), Some(Component::Normal(_)))
                && components.next().is_none()
                && !name.to_string_lossy().contains(['/', '\\', '\0']))
        {
            return Err(MftPathError::InvalidName);
        }
        self.entries.insert(
            number,
            Node {
                reference,
                parent,
                name: name.to_owned(),
            },
        );
        Ok(true)
    }
    /// Reconstruct a root-relative path, checking every full reference.
    /// Native path conversion uses [`NtfsName::to_os_string`], which replaces
    /// unpaired surrogates on Unix. Exact quality describes reference matching.
    pub fn resolve_strict(&self, reference: NtfsFileReference) -> Result<PathBuf, MftPathError> {
        let result = self.walk(reference, false);
        match result.error {
            Some(error) => Err(error),
            None => Ok(result.path),
        }
    }
    /// Reconstruct as much as possible, explicitly allowing record-number
    /// fallback. Partial paths are suffixes and must not be treated as rooted.
    pub fn resolve_best_effort(&self, reference: NtfsFileReference) -> MftPathResolution {
        self.walk(reference, true)
    }

    fn walk(&self, mut reference: NtfsFileReference, allow_fallback: bool) -> MftPathResolution {
        let mut names = Vec::new();
        let mut seen = FxHashSet::default();
        let mut fallback = false;
        let mut failure = Some(MftPathError::DepthLimit);
        for _ in 0..256 {
            let number = reference.record_number();
            if !seen.insert(number) {
                failure = Some(MftPathError::Cycle(reference));
                break;
            }
            let Some(node) = self.entries.get(&number) else {
                failure = Some(MftPathError::MissingReference(reference));
                break;
            };
            if node.reference != reference {
                if !allow_fallback {
                    failure = Some(MftPathError::SequenceMismatch {
                        requested: reference,
                        indexed: node.reference,
                    });
                    break;
                }
                fallback = true;
            }
            if number == 5 && node.parent == node.reference {
                failure = None;
                break;
            }
            names.push(node.name.to_os_string());
            reference = node.parent;
        }
        let path = names.into_iter().rev().collect();
        MftPathResolution {
            path,
            quality: if failure.is_some() {
                MftPathQuality::Partial
            } else if fallback {
                MftPathQuality::RecordFallback
            } else {
                MftPathQuality::Exact
            },
            used_record_fallback: fallback,
            error: failure,
        }
    }
}
