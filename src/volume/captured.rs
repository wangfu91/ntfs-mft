//! Test-only volume source retaining captured bytes at their original offsets.
//! Reads outside captured ranges fail; omitted disk regions are never zero-filled.
use super::{Volume, VolumeSource};
use crate::MftError;
use std::{io, sync::Arc};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct CapturedVolume {
    ranges: Vec<(u64, Arc<[u8]>)>,
}

impl CapturedVolume {
    pub(crate) fn new(mut ranges: Vec<(u64, Vec<u8>)>) -> Self {
        ranges.sort_by_key(|(offset, _)| *offset);
        let mut previous_end = 0;
        for (offset, bytes) in &ranges {
            assert!(!bytes.is_empty());
            assert!(*offset >= previous_end, "overlapping captured ranges");
            previous_end = offset.checked_add(bytes.len() as u64).unwrap();
        }
        Self {
            ranges: ranges
                .into_iter()
                .map(|(offset, bytes)| (offset, bytes.into()))
                .collect(),
        }
    }

    pub(crate) fn open(self: &Arc<Self>) -> Result<Volume, MftError> {
        let source = VolumeSource::Captured(Arc::clone(self));
        #[cfg(windows)]
        let volume = Volume::mock(windows::Win32::Foundation::HANDLE::default(), source);
        #[cfg(not(windows))]
        let volume = {
            // Keep the normal File ownership shape without opening any device.
            // VolumeReader routes every read to the captured source instead.
            let mut volume = Volume::from_device_path(std::env::current_exe()?)?;
            volume.source = source;
            volume
        };
        Ok(volume)
    }

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
            format!("uncaptured volume offset {offset}"),
        ))
    }
}
