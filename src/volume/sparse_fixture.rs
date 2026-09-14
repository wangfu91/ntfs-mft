//! Test-only sparse volume fixture.
//!
//! The fixture retains bytes only for explicitly supplied volume-offset ranges.
//! A read outside those ranges fails instead of returning synthetic zero bytes,
//! so tests cannot mistake incomplete capture data for a complete volume.

use super::{Volume, VolumeSource};
use crate::MftError;
use std::{io, sync::Arc};

/// An immutable sparse fixture for testing random reads from a volume.
///
/// This is not a production offline-volume API. It models a partial capture:
/// each range has its original volume offset and any omitted offset is an I/O
/// error. Shared ownership lets parallel-reader tests reopen the same fixture
/// without copying its byte ranges.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct SparseVolumeFixture {
    ranges: Vec<(u64, Arc<[u8]>)>,
}

impl SparseVolumeFixture {
    /// Create a fixture from nonempty, non-overlapping `(volume_offset, bytes)` ranges.
    ///
    /// Ranges are sorted by offset. Invalid fixture data is a test-authoring
    /// error and panics rather than creating ambiguous read behavior.
    pub(crate) fn new(mut ranges: Vec<(u64, Vec<u8>)>) -> Self {
        ranges.sort_by_key(|(offset, _)| *offset);
        let mut previous_end = 0;
        for (offset, bytes) in &ranges {
            assert!(!bytes.is_empty());
            assert!(*offset >= previous_end, "overlapping fixture ranges");
            previous_end = offset.checked_add(bytes.len() as u64).unwrap();
        }
        Self {
            ranges: ranges
                .into_iter()
                .map(|(offset, bytes)| (offset, bytes.into()))
                .collect(),
        }
    }

    /// Open this fixture through the ordinary test-only `Volume` read path.
    ///
    /// The resulting volume routes reads to this fixture before touching its
    /// platform-specific handle or file placeholder.
    pub(crate) fn open(self: &Arc<Self>) -> Result<Volume, MftError> {
        let source = VolumeSource::SparseFixture(Arc::clone(self));
        #[cfg(windows)]
        let volume = Volume::mock(windows::Win32::Foundation::HANDLE::default(), source);
        #[cfg(not(windows))]
        let volume = {
            // Keep the normal File ownership shape without opening any device.
            // VolumeReader routes every read to the sparse fixture instead.
            let mut volume = Volume::from_device_path(std::env::current_exe()?)?;
            volume.source = source;
            volume
        };
        Ok(volume)
    }

    /// Read bytes from one supplied range at its original volume offset.
    ///
    /// Reads never span two ranges. An offset outside every range returns
    /// `UnexpectedEof`, including intentionally omitted all-zero regions.
    pub(crate) fn read_at(&self, out: &mut [u8], offset: u64) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        for (start, bytes) in &self.ranges {
            if offset >= *start && offset - start < bytes.len() as u64 {
                let index = (offset - start) as usize;
                let count = out.len().min(bytes.len() - index);
                out[..count].copy_from_slice(&bytes[index..index + count]);
                return Ok(count);
            }
        }
        Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            format!("omitted sparse-fixture offset {offset}"),
        ))
    }
}
