//! Snapshot path resolver for entries produced by a raw `$MFT` scan.

#[cfg(windows)]
use std::cell::RefCell;
use std::path::{Path, PathBuf};

#[cfg(windows)]
use crate::path::resolve::resolve_path as resolve_live_path;
use crate::{MftPathIndex, MftResult, volume::Volume};

use super::{RawMft, RawMftEntry};

/// Path resolver specialized for entries produced by [`RawMft`].
///
/// Default lookups check full file references in an in-memory index. Building
/// this resolver performs a separate scan, not an atomic filesystem snapshot.
/// Use [`Self::from_index`] to reuse an index populated during an existing scan.
/// Windows live fallback is explicitly opt-in and may observe newer state.
#[derive(Debug)]
pub struct RawMftPathResolver<'a> {
    /// Volume from which the raw `$MFT` snapshot was built.
    volume: &'a Volume,
    /// Snapshot-local directory tree built from the raw `$MFT` scan.
    index: MftPathIndex,
    /// Whether live `OpenFileById` fallback is allowed when the snapshot tree misses.
    #[cfg(windows)]
    live_fallback: bool,
    /// Reusable heap buffer for optional live fallback lookups.
    #[cfg(windows)]
    buffer: RefCell<Vec<u8>>,
}

impl<'a> RawMftPathResolver<'a> {
    /// Scan all base records, including the root, and propagate scan I/O errors.
    pub(crate) fn new(raw_mft: &RawMft<'a>) -> MftResult<Self> {
        let mut index = MftPathIndex::new();
        let options = super::RawMftScanOptions::builder().start_record(0).build();
        for result in raw_mft.scan_entries_with_options(options)? {
            let entry = result?;
            if entry.is_used {
                index.insert(&entry)?;
            }
        }
        Ok(Self::from_index(raw_mft.volume(), index))
    }

    /// Attach a volume prefix and optional live fallback to a caller-built index.
    /// Performs no scan. The index should include root record 5.
    pub fn from_index(volume: &'a Volume, index: MftPathIndex) -> Self {
        Self {
            volume,
            index,
            #[cfg(windows)]
            live_fallback: false,
            #[cfg(windows)]
            buffer: RefCell::new(Vec::new()),
        }
    }
    /// Inspect the index or perform strict and best-effort relative lookups.
    pub fn index(&self) -> &MftPathIndex {
        &self.index
    }
    /// Recover the volume-independent index without I/O.
    pub fn into_index(self) -> MftPathIndex {
        self.index
    }

    /// Enable best-effort live fallback for entries missing from the snapshot tree.
    ///
    /// This is opt-in because live fallback mixes current filesystem state into
    /// results derived from previously scanned records.
    #[must_use]
    #[cfg(windows)]
    pub fn with_live_fallback(mut self) -> Self {
        self.live_fallback = true;
        self
    }

    /// Resolve `entry` using full-reference checks. Use `index().resolve_strict`
    /// to inspect the reason for a missing indexed path.
    ///
    /// On Windows, when `with_live_fallback` has been enabled, unresolved
    /// snapshot entries fall back to `OpenFileById` against the mounted volume.
    #[must_use]
    pub fn resolve_path(&self, entry: &RawMftEntry) -> Option<PathBuf> {
        let snapshot_path = self
            .index
            .resolve_strict(entry.file_reference)
            .ok()
            .map(|path| prefix_mount_point(self.volume, &path));
        #[cfg(windows)]
        return snapshot_path.or_else(|| {
            self.live_fallback.then(|| {
                resolve_live_path(
                    self.volume,
                    entry.file_reference,
                    entry.parent_reference,
                    entry.file_name.to_os_string().as_os_str(),
                    &self.buffer,
                )
            })?
        });
        #[cfg(not(windows))]
        snapshot_path
    }
}

/// Snapshot tree paths already include drive-letter roots. For volumes opened
/// through a mount point the tree returns a relative path, so attach that
/// source prefix before exposing the result.
fn prefix_mount_point(volume: &Volume, path: &Path) -> PathBuf {
    if let Some(root) = volume.mount_point() {
        return root.join(path);
    }
    if let Some(drive) = volume.drive_letter() {
        return PathBuf::from(format!("{}:\\", drive.to_ascii_uppercase())).join(path);
    }
    path.to_path_buf()
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::volume::VolumeSource;
    use windows::Win32::Foundation::HANDLE;

    #[test]
    fn snapshot_path_is_prefixed_for_mount_point_volumes() {
        let volume = Volume::mock(
            HANDLE(std::ptr::null_mut()),
            VolumeSource::MountPoint(PathBuf::from(r"C:\mnt\data")),
        );

        assert_eq!(
            prefix_mount_point(&volume, Path::new(r"dir\file.txt")),
            PathBuf::from(r"C:\mnt\data\dir\file.txt")
        );
    }

    #[test]
    fn relative_index_path_receives_drive_prefix() {
        let volume = Volume::mock(HANDLE(std::ptr::null_mut()), VolumeSource::DriveLetter('C'));
        let path = Path::new(r"dir\file.txt");
        assert_eq!(
            prefix_mount_point(&volume, path),
            Path::new(r"C:\dir\file.txt")
        );
    }
}
