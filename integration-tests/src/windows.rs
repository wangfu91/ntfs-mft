//! Read-only FSCTL enumeration for raw-record parity tests.
use crate::{MftEntry, parse_record};
use ntfs_mft::MftError;
use std::{collections::VecDeque, io, mem::size_of};
use windows::{
    Win32::{
        Foundation::{CloseHandle, ERROR_ACCESS_DENIED, ERROR_HANDLE_EOF, HANDLE},
        Storage::FileSystem::*,
        System::{
            IO::DeviceIoControl,
            Ioctl::{FSCTL_ENUM_USN_DATA, MFT_ENUM_DATA_V1},
        },
    },
    core::HSTRING,
};

struct OwnedHandle(HANDLE);
impl Drop for OwnedHandle {
    fn drop(&mut self) {
        // SAFETY: this wrapper uniquely owns a successfully opened handle.
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

pub struct Volume {
    handle: OwnedHandle,
}
impl Volume {
    pub fn from_drive_letter(drive: char) -> Result<Self, MftError> {
        // SAFETY: the path is valid for this call; no asynchronous IO is requested.
        let handle = unsafe {
            CreateFileW(
                &HSTRING::from(format!(r"\\.\{drive}:")),
                FILE_GENERIC_READ.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                None,
                OPEN_EXISTING,
                FILE_FLAGS_AND_ATTRIBUTES::default(),
                None,
            )
        };
        match handle {
            Ok(handle) => Ok(Self {
                handle: OwnedHandle(handle),
            }),
            Err(e) if e.code() == ERROR_ACCESS_DENIED.into() => Err(MftError::NotElevated),
            Err(e) => Err(e.into()),
        }
    }
}

pub struct Mft<'a> {
    volume: &'a Volume,
}
impl<'a> Mft<'a> {
    pub fn new(volume: &'a Volume) -> Self {
        Self { volume }
    }
    pub fn try_iter(&self) -> io::Result<MftIter<'a>> {
        let mut iter = MftIter {
            volume: self.volume,
            request: MFT_ENUM_DATA_V1 {
                StartFileReferenceNumber: 0,
                LowUsn: 0,
                HighUsn: i64::MAX,
                MinMajorVersion: 2,
                // Pin V2 for standard 64-bit NTFS identities in parity checks.
                MaxMajorVersion: 2,
            },
            entries: VecDeque::new(),
            done: false,
            buffer: vec![0; 64 * 1024],
        };
        iter.refill()?;
        Ok(iter)
    }
}

pub struct MftIter<'a> {
    volume: &'a Volume,
    request: MFT_ENUM_DATA_V1,
    entries: VecDeque<MftEntry>,
    done: bool,
    buffer: Vec<u8>,
}
impl MftIter<'_> {
    fn refill(&mut self) -> io::Result<()> {
        let mut returned = 0;
        // SAFETY: the handle is borrowed; input and output buffers are valid for
        // their specified sizes and remain alive until synchronous IO completes.
        let result = unsafe {
            DeviceIoControl(
                self.volume.handle.0,
                FSCTL_ENUM_USN_DATA,
                Some((&self.request as *const MFT_ENUM_DATA_V1).cast()),
                size_of::<MFT_ENUM_DATA_V1>() as u32,
                Some(self.buffer.as_mut_ptr().cast()),
                self.buffer.len() as u32,
                Some(&mut returned),
                None,
            )
        };
        if let Err(e) = result {
            if e.code() == ERROR_HANDLE_EOF.into() {
                self.done = true;
                return Ok(());
            }
            return Err(io::Error::other(e));
        }
        let bytes = self
            .buffer
            .get(..returned as usize)
            .filter(|b| b.len() >= 8)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "missing FSCTL continuation cursor",
                )
            })?;
        let next = u64::from_le_bytes(bytes[..8].try_into().unwrap());
        if next <= self.request.StartFileReferenceNumber {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "FSCTL cursor did not advance",
            ));
        }
        let mut remaining = &bytes[8..];
        while !remaining.is_empty() {
            let (entry, length) = parse_record(remaining)?;
            self.entries.push_back(entry);
            remaining = &remaining[length..];
        }
        self.request.StartFileReferenceNumber = next;
        Ok(())
    }
}
impl Iterator for MftIter<'_> {
    type Item = io::Result<MftEntry>;
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(entry) = self.entries.pop_front() {
                return Some(Ok(entry));
            }
            if self.done {
                return None;
            }
            if let Err(e) = self.refill() {
                self.done = true;
                self.entries.clear();
                return Some(Err(e));
            }
        }
    }
}
