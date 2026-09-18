//! Sector-aligned reader over a raw NTFS volume handle or Unix file.
//!
//! Raw volume reads via `ReadFile` must be sector-aligned in both offset
//! and length on Windows. NTFS FILE records are typically 1 KiB while
//! sectors are 512 bytes (or 4 KiB on Advanced Format drives), so we wrap
//! the raw volume handle in a `Read + Seek` adapter that internally
//! buffers a configurable number of sectors. Higher-level code (the MFT
//! iterator) then sees a familiar byte-oriented stream.

use crate::errors::MftError;
use crate::volume::Volume;
use std::io::{self, Read, Seek, SeekFrom};
use std::num::NonZeroUsize;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::os::unix::fs::FileExt;
#[cfg(windows)]
use windows::Win32::{
    Foundation::{ERROR_HANDLE_EOF, ERROR_IO_PENDING, HANDLE},
    Storage::FileSystem::ReadFile,
    System::{
        IO::{GetOverlappedResult, OVERLAPPED},
        Threading::CreateEventW,
    },
};

/// Reads from a borrowed Windows volume handle or an owned Unix file clone.
///
/// The reader maintains its own byte-level cursor; every read is broken
/// into sector-aligned `ReadFile` calls under the hood, with the data
/// copied into the caller's buffer.
pub(crate) struct VolumeReader {
    #[cfg(test)]
    sparse_fixture: Option<std::sync::Arc<crate::volume::sparse_fixture::SparseVolumeFixture>>,
    /// Borrowed raw volume handle.
    #[cfg(windows)]
    handle: HANDLE,
    /// One completion event per reader; at most one outstanding request uses its buffer.
    #[cfg(windows)]
    event: std::os::windows::io::OwnedHandle,
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    file: std::fs::File,
    /// Required sector alignment for I/O.
    sector_size: u64,
    /// Byte-oriented cursor exposed through `Read + Seek`.
    position: u64,
    /// Sector-aligned backing buffer.
    buf: Vec<u8>,
    /// Volume offset of the start of `buf`.
    buf_pos: u64,
    /// Number of valid bytes in `buf`.
    buf_len: usize,
}

impl VolumeReader {
    /// Create a reader with the requested internal buffer size.
    pub(crate) fn new(
        volume: &Volume,
        sector_size: u64,
        buffer_bytes: NonZeroUsize,
    ) -> Result<Self, MftError> {
        if !sector_size.is_power_of_two() || sector_size == 0 {
            return Err(MftError::InvalidBootSector(
                "sector_size must be a non-zero power of two",
            ));
        }
        // The device may require larger reads than the NTFS sector size,
        // including during bootstrap before the NTFS geometry is known.
        #[cfg(target_os = "macos")]
        let sector_size = sector_size.max(volume.io_alignment);

        let sectors = (buffer_bytes.get() as u64 / sector_size).max(1);
        let cap = (sectors * sector_size) as usize;
        #[cfg(windows)]
        let event = {
            use std::os::windows::io::FromRawHandle;
            // SAFETY: unnamed manual-reset event, no borrowed security/name pointers.
            let event = unsafe { CreateEventW(None, true, false, None) }?;
            // SAFETY: transfer the newly created event to its unique RAII owner.
            unsafe { std::os::windows::io::OwnedHandle::from_raw_handle(event.0) }
        };
        Ok(Self {
            #[cfg(test)]
            sparse_fixture: volume.sparse_fixture().cloned(),
            #[cfg(windows)]
            handle: volume.handle,
            #[cfg(windows)]
            event,
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            file: volume.file.try_clone()?,
            sector_size,
            position: 0,
            buf: vec![0u8; cap],
            buf_pos: u64::MAX,
            buf_len: 0,
        })
    }

    /// Round `n` down to the start of its containing sector.
    ///
    /// `sector_size` is validated at construction to be a non-zero power of
    /// two. For such a size, the low bits of `n` encode its byte offset within
    /// a sector. `sector_size - 1` has exactly those low bits set, so its
    /// inverse keeps the sector-number bits and clears the intra-sector
    /// offset. Applying that mask returns the sector boundary at or before
    /// `n`: with 512-byte sectors, 513 becomes 512 while 512 remains 512.
    fn round_down(&self, n: u64) -> u64 {
        n & !(self.sector_size - 1)
    }

    /// Refill the internal buffer starting at a sector-aligned volume offset.
    fn refill(&mut self, sector_pos: u64) -> io::Result<()> {
        #[cfg(test)]
        if let Some(source) = &self.sparse_fixture {
            let count = source.read_at(&mut self.buf, sector_pos)?;
            self.buf_pos = sector_pos;
            self.buf_len = count;
            return Ok(());
        }

        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            let bytes_read = self.file.read_at(&mut self.buf, sector_pos)?;
            if bytes_read == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "zero-length read from volume",
                ));
            }
            self.buf_pos = sector_pos;
            self.buf_len = bytes_read;
            Ok(())
        }

        #[cfg(windows)]
        {
            let mut bytes_read: u32 = 0;
            use std::os::windows::io::AsRawHandle;
            let mut overlapped = OVERLAPPED {
                hEvent: HANDLE(self.event.as_raw_handle()),
                ..OVERLAPPED::default()
            };
            overlapped.Anonymous.Anonymous.Offset = sector_pos as u32;
            overlapped.Anonymous.Anonymous.OffsetHigh = (sector_pos >> 32) as u32;
            // Explicit offsets also support duplicated handles sharing a file
            // object. Each reader has its own event and only one request in flight.
            // SAFETY: the handle is borrowed from a live Volume. The sector-aligned
            // buffer, OVERLAPPED and event stay live and untouched until this
            // request completes, including the pending asynchronous path below.
            let res = unsafe {
                ReadFile(
                    self.handle,
                    Some(self.buf.as_mut_slice()),
                    None,
                    Some(&mut overlapped),
                )
            };
            let res = match res {
                Ok(()) => unsafe {
                    GetOverlappedResult(self.handle, &overlapped, &mut bytes_read, false)
                },
                Err(error) if error == ERROR_IO_PENDING.into() => {
                    // SAFETY: wait for this reader's event, never another worker's
                    // completion on the shared file object. Do not release or
                    // reuse the buffer/OVERLAPPED while the kernel may access it.
                    unsafe { GetOverlappedResult(self.handle, &overlapped, &mut bytes_read, true) }
                }
                Err(error) => Err(error),
            };
            res.map_err(|error| {
                if error == ERROR_HANDLE_EOF.into() {
                    io::Error::new(io::ErrorKind::UnexpectedEof, "zero-length read from volume")
                } else {
                    io::Error::other(error)
                }
            })?;
            if bytes_read == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "zero-length read from volume",
                ));
            }
            self.buf_pos = sector_pos;
            self.buf_len = bytes_read as usize;
            Ok(())
        }
    }

    /// Return whether the internal buffer fully covers the requested range.
    fn buf_contains(&self, position: u64, n: usize) -> bool {
        if self.buf_len == 0 {
            return false;
        }
        let Some(buf_end) = self.buf_pos.checked_add(self.buf_len as u64) else {
            return false;
        };
        position >= self.buf_pos
            && position
                .checked_add(n as u64)
                .is_some_and(|end| end <= buf_end)
    }

    /// Return a mutable view of `len` bytes at the given volume offset
    /// directly out of the internal buffer, refilling if necessary.
    ///
    /// This skips the per-record memcpy that `Read::read_exact` would
    /// otherwise perform — the caller can fix up and parse the record
    /// in place. Safe because each MFT record's USA fixup only touches
    /// bytes inside that record.
    ///
    /// Unlike [`Read::read`], this absolute-offset access does not change the
    /// byte-oriented stream cursor.
    pub(crate) fn buffer_at_mut(&mut self, offset: u64, len: usize) -> io::Result<&mut [u8]> {
        if !self.buf_contains(offset, len) {
            // If the requested range can't fit in the buffer at all,
            // grow the buffer to a sector-aligned size that holds it.
            let sector_pos = self.round_down(offset);
            let needed = (offset - sector_pos)
                .checked_add(len as u64)
                .ok_or_else(|| io::Error::other("borrow buffer size overflow"))?
                .div_ceil(self.sector_size)
                .checked_mul(self.sector_size)
                .ok_or_else(|| io::Error::other("borrow buffer size overflow"))?;
            let needed = usize::try_from(needed)
                .map_err(|_| io::Error::other("borrow buffer size exceeds addressable memory"))?;
            if needed > self.buf.len() {
                self.buf.resize(needed, 0);
            }
            self.refill(sector_pos)?;
            if !self.buf_contains(offset, len) {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "short read while borrowing volume buffer",
                ));
            }
        }
        let inner_off = (offset - self.buf_pos) as usize;
        Ok(&mut self.buf[inner_off..inner_off + len])
    }
}

impl Read for VolumeReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        // Fast path: requested bytes already in the buffered window.
        if !self.buf_contains(self.position, 1) {
            let sector_pos = self.round_down(self.position);
            self.refill(sector_pos)?;
        }
        let inner_off = (self.position - self.buf_pos) as usize;
        let avail = self.buf_len - inner_off;
        let n = out.len().min(avail);
        out[..n].copy_from_slice(&self.buf[inner_off..inner_off + n]);
        self.position += n as u64;
        Ok(n)
    }
}

impl Seek for VolumeReader {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let new_pos = match pos {
            SeekFrom::Start(n) => n,
            SeekFrom::Current(n) => {
                if n >= 0 {
                    self.position
                        .checked_add(n as u64)
                        .ok_or_else(|| io::Error::other("seek overflow"))?
                } else {
                    self.position
                        .checked_sub((-n) as u64)
                        .ok_or_else(|| io::Error::other("seek underflow"))?
                }
            }
            SeekFrom::End(_) => {
                return Err(io::Error::other("seek from end not supported"));
            }
        };
        self.position = new_pos;
        Ok(new_pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buffer_bytes(value: usize) -> NonZeroUsize {
        NonZeroUsize::new(value).expect("test buffer size must be non-zero")
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn positional_readers_keep_independent_cursors_and_own_their_files() {
        #[cfg(not(ntfs_mft_differential))]
        let fixture = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/omerbenamram-mft/MFT"
        );
        // The differential harness compiles this source from its own manifest.
        #[cfg(ntfs_mft_differential)]
        let fixture = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../tests/fixtures/omerbenamram-mft/MFT"
        );
        let expected = std::fs::read(fixture).unwrap();
        let volume = Volume::from_device_path(fixture).unwrap();
        let mut first = VolumeReader::new(&volume, 512, buffer_bytes(512)).unwrap();
        let mut second = VolumeReader::new(&volume, 512, buffer_bytes(512)).unwrap();
        drop(volume);
        let mut bytes = [0; 1024];
        for offset in [4093, 5, 1025, 512] {
            second.seek(SeekFrom::Start(offset + 8192)).unwrap();
            second.read_exact(&mut bytes).unwrap();
            assert_eq!(
                &bytes,
                &expected[offset as usize + 8192..offset as usize + 9216]
            );
            first.seek(SeekFrom::Start(offset)).unwrap();
            first.read_exact(&mut bytes).unwrap();
            assert_eq!(&bytes, &expected[offset as usize..offset as usize + 1024]);
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn device_alignment_covers_bootstrap_and_cross_block_record_buffers() {
        #[cfg(not(ntfs_mft_differential))]
        let fixture = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/omerbenamram-mft/MFT"
        );
        #[cfg(ntfs_mft_differential)]
        let fixture = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../tests/fixtures/omerbenamram-mft/MFT"
        );
        let expected = std::fs::read(fixture).unwrap();
        let mut volume = Volume::from_device_path(fixture).unwrap();
        // Simulate a 4 KiB device carrying 512-byte NTFS sectors. The fixture
        // permits deterministic read verification without physical 4Kn media.
        volume.io_alignment = 4096;
        let mut reader = VolumeReader::new(&volume, 512, buffer_bytes(512)).unwrap();
        assert_eq!(reader.sector_size, 4096);
        assert_eq!(reader.buf.len(), 4096);
        let mut boot = [0; 512];
        reader.read_exact(&mut boot).unwrap();
        assert_eq!(&boot, &expected[..512]);
        assert_eq!(
            reader.buffer_at_mut(4093, 1024).unwrap(),
            &expected[4093..5117]
        );
        assert_eq!(reader.stream_position().unwrap(), 512);
        assert_eq!(reader.buf_pos, 0);
        assert_eq!(reader.buf.len(), 8192);
        assert_eq!(
            reader.buffer_at_mut(12285, 1024).unwrap(),
            &expected[12285..13309]
        );
        assert_eq!(reader.stream_position().unwrap(), 512);
        assert_eq!(reader.buf_pos, 8192);
    }

    fn reader() -> VolumeReader {
        #[cfg(windows)]
        let volume = crate::volume::Volume::mock(
            HANDLE(std::ptr::null_mut()),
            crate::volume::VolumeSource::DriveLetter('C'),
        );
        #[cfg(target_os = "linux")]
        let volume =
            crate::volume::Volume::from_device_path("/dev/zero").expect("open test byte source");
        #[cfg(target_os = "macos")]
        let volume = crate::volume::Volume::from_device_path(std::env::current_exe().unwrap())
            .expect("open test byte source");
        VolumeReader::new(&volume, 512, buffer_bytes(256 * 1024)).expect("construct test reader")
    }

    #[test]
    fn round_down_aligns_to_sector() {
        let r = reader();
        assert_eq!(r.round_down(0), 0);
        assert_eq!(r.round_down(511), 0);
        assert_eq!(r.round_down(512), 512);
        assert_eq!(r.round_down(513), 512);
        assert_eq!(r.round_down(1023), 512);
        assert_eq!(r.round_down(4096), 4096);
    }

    #[test]
    fn rejects_non_power_of_two_sector_size() {
        #[cfg(windows)]
        let volume = crate::volume::Volume::mock(
            HANDLE(std::ptr::null_mut()),
            crate::volume::VolumeSource::DriveLetter('C'),
        );
        #[cfg(target_os = "linux")]
        let volume =
            crate::volume::Volume::from_device_path("/dev/zero").expect("open test byte source");
        #[cfg(target_os = "macos")]
        let volume = crate::volume::Volume::from_device_path(std::env::current_exe().unwrap())
            .expect("open test byte source");
        assert!(VolumeReader::new(&volume, 0, buffer_bytes(512)).is_err());
        assert!(VolumeReader::new(&volume, 3, buffer_bytes(512)).is_err());
        assert!(VolumeReader::new(&volume, 6, buffer_bytes(512)).is_err());
        assert!(VolumeReader::new(&volume, 512, buffer_bytes(512)).is_ok());
    }

    #[test]
    fn buf_contains_handles_window() {
        let mut r = reader();
        r.buf_pos = 1024;
        r.buf_len = 2048;
        assert!(r.buf_contains(1024, 1));
        assert!(r.buf_contains(1024, 2048));
        assert!(!r.buf_contains(1024, 2049));
        assert!(!r.buf_contains(1023, 1));
        assert!(r.buf_contains(3071, 1));
        assert!(!r.buf_contains(3072, 1));
        r.buf_len = 0;
        assert!(!r.buf_contains(1024, 1));
    }
}

#[cfg(all(test, windows))]
mod shared_handle_tests {
    use super::*;
    use std::os::windows::io::IntoRawHandle;

    #[test]
    fn shared_file_object_reads_use_independent_explicit_offsets() {
        for flags in [
            0,
            windows::Win32::Storage::FileSystem::FILE_FLAG_OVERLAPPED.0,
        ] {
            #[cfg(not(ntfs_mft_differential))]
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/sparse-volume-fixture/mft.bin");
            #[cfg(ntfs_mft_differential)]
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../tests/fixtures/sparse-volume-fixture/mft.bin");
            let expected = std::fs::read(&path).unwrap();
            use std::os::windows::fs::OpenOptionsExt;
            let file = std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(flags)
                .open(&path)
                .unwrap();
            let volume = Volume::mock(
                HANDLE(file.into_raw_handle()),
                crate::volume::VolumeSource::MountPoint(path),
            );
            let source = volume.shared_read_source().unwrap();
            drop(volume); // The source must independently own a live duplicate.
            std::thread::scope(|scope| {
                for worker in 0..8 {
                    let source = source.clone();
                    let expected = &expected;
                    scope.spawn(move || {
                        let volume = source.open().unwrap();
                        let mut reader =
                            VolumeReader::new(&volume, 512, NonZeroUsize::new(4096).unwrap())
                                .unwrap();
                        for index in 0..256 {
                            let offset = (index * 997 + worker * 173) % (expected.len() - 1024);
                            assert_eq!(
                                reader.buffer_at_mut(offset as u64, 1024).unwrap(),
                                &expected[offset..offset + 1024]
                            );
                        }
                        reader.seek(SeekFrom::Start(expected.len() as u64)).unwrap();
                        assert_eq!(
                            reader.read_exact(&mut [0u8; 1]).unwrap_err().kind(),
                            io::ErrorKind::UnexpectedEof
                        );
                    });
                }
            });
        }
    }
}
