//! Read-only volume access for NTFS/ReFS on Windows and raw NTFS on Linux and macOS.

#[cfg(test)]
pub(crate) mod sparse_fixture;

use crate::errors::MftError;
#[cfg(windows)]
use crate::privilege;
#[cfg(windows)]
use log::{debug, warn};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::fs::{File, OpenOptions};
#[cfg(target_os = "linux")]
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos::device_for_mount_point;
#[cfg(windows)]
use windows::{
    Win32::{
        Foundation::{CloseHandle, ERROR_ACCESS_DENIED, HANDLE},
        Storage::FileSystem::{
            CreateFileW, FILE_FLAG_OVERLAPPED, FILE_GENERIC_READ, FILE_SHARE_READ,
            FILE_SHARE_WRITE, GetVolumeNameForVolumeMountPointW, OPEN_EXISTING,
        },
    },
    core::HSTRING,
};

/// Source used to open a [`Volume`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum VolumeSource {
    /// Test-only sparse fixture; omitted offsets fail instead of reading as zeroes.
    #[cfg(test)]
    SparseFixture(std::sync::Arc<sparse_fixture::SparseVolumeFixture>),
    /// Volume was opened via a drive letter (e.g. `'C'`).
    #[cfg(windows)]
    DriveLetter(char),
    /// Volume was opened via a mount point path.
    MountPoint(PathBuf),
    /// Volume was opened directly through a Unix device path.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    DevicePath(PathBuf),
}

#[derive(Debug)]
/// Represents a read-only filesystem volume and its reopenable source.
pub struct Volume {
    /// Raw volume handle returned by `CreateFileW`.
    ///
    /// This crate owns the handle and closes it exactly once in [`Drop`].
    /// Internal users may borrow it for Win32 calls but must never close,
    /// transfer ownership of it. Scan-local duplicates are separately owned and
    /// require explicit-offset reads because they share the OS file cursor.
    #[cfg(windows)]
    pub(crate) handle: HANDLE,
    /// Read-only Unix device file.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(crate) file: File,
    /// Logical device-block alignment required by macOS raw reads.
    #[cfg(target_os = "macos")]
    pub(crate) io_alignment: u64,
    /// Source path used to open the volume.
    source: VolumeSource,
}

impl Volume {
    #[cfg(test)]
    pub(crate) fn sparse_fixture(
        &self,
    ) -> Option<&std::sync::Arc<sparse_fixture::SparseVolumeFixture>> {
        match &self.source {
            VolumeSource::SparseFixture(source) => Some(source),
            _ => None,
        }
    }

    /// Reopen the same source read-only with an independent OS file cursor.
    pub(crate) fn reopen(&self) -> Result<Self, MftError> {
        match &self.source {
            #[cfg(test)]
            VolumeSource::SparseFixture(source) => source.open(),
            #[cfg(windows)]
            VolumeSource::DriveLetter(letter) => Self::from_drive_letter(*letter),
            VolumeSource::MountPoint(path) => Self::from_mount_point(path),
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            VolumeSource::DevicePath(path) => Self::from_device_path(path),
        }
    }

    /// Creates a new `Volume` instance with the given drive letter.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ntfs_mft::volume::Volume;
    ///
    /// // Requires Administrator privileges.
    /// let volume = Volume::from_drive_letter('C')?;
    /// for result in volume.raw_mft()?.scan_entries()?.take(10) {
    ///     println!("{}", result?);
    /// }
    /// # Ok::<(), ntfs_mft::MftError>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`MftError::NotElevated`] if the process does not have the
    /// Administrator privileges required to open raw volume handles. Returns
    /// [`MftError::WinApi`] if Windows rejects the volume path.
    #[cfg(windows)]
    pub fn from_drive_letter(drive_letter: char) -> Result<Self, MftError> {
        let handle = get_volume_handle_from_drive_letter(drive_letter)?;
        Ok(Volume {
            handle,
            source: VolumeSource::DriveLetter(drive_letter),
        })
    }

    /// Creates a new `Volume` instance with the given mount point.
    ///
    /// # Errors
    ///
    /// Returns [`MftError::NotElevated`] if the process does not have the
    /// Administrator privileges required to open raw volume handles. Returns
    /// [`MftError::WinApi`] or [`MftError::InvalidMountPoint`] if the
    /// mount point cannot be resolved to a volume handle.
    #[cfg(windows)]
    pub fn from_mount_point<P: AsRef<Path>>(mount_point: P) -> Result<Self, MftError> {
        let path = mount_point.as_ref();
        let handle = get_volume_handle_from_mount_point(path)?;
        Ok(Volume {
            handle,
            source: VolumeSource::MountPoint(path.to_path_buf()),
        })
    }

    /// Opens the device backing an NTFS mount on Linux or macOS, read-only.
    ///
    /// The path must name the mount root, not a child directory. On macOS, the
    /// native `ntfs` filesystem type is required and the backing BSD disk is
    /// opened through its raw character interface. OS access errors are returned
    /// as [`MftError::Io`].
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub fn from_mount_point<P: AsRef<Path>>(mount_point: P) -> Result<Self, MftError> {
        let mount_point = mount_point.as_ref().canonicalize()?;
        let device = device_for_mount_point(&mount_point)?;
        let file = open_read_only(&device)?;
        Ok(Self {
            #[cfg(target_os = "macos")]
            io_alignment: macos::device_alignment(&file)?,
            file,
            source: VolumeSource::MountPoint(mount_point),
        })
    }

    /// Opens an NTFS device or partition image on Linux or macOS read-only.
    ///
    /// Byte zero must be the NTFS boot sector, not a whole-disk partition table.
    /// On macOS, `/dev/diskNsM` is opened through its `/dev/rdiskNsM` raw
    /// character interface so mounted partitions remain readable. Either path
    /// is accepted; [`Self::device_path`] retains the caller-supplied path.
    /// Filesystem validation happens when constructing [`crate::RawMft`].
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub fn from_device_path<P: AsRef<Path>>(device_path: P) -> Result<Self, MftError> {
        let path = device_path.as_ref().to_path_buf();
        let file = open_read_only(&path)?;
        Ok(Self {
            #[cfg(target_os = "macos")]
            io_alignment: macos::device_alignment(&file)?,
            file,
            source: VolumeSource::DevicePath(path),
        })
    }

    /// Returns the drive letter if this volume was opened via a drive letter.
    #[must_use]
    #[inline]
    pub fn drive_letter(&self) -> Option<char> {
        match &self.source {
            #[cfg(windows)]
            VolumeSource::DriveLetter(c) => Some(*c),
            _ => None,
        }
    }

    /// Returns the mount point path if this volume was opened via a mount point.
    #[must_use]
    #[inline]
    pub fn mount_point(&self) -> Option<&Path> {
        match &self.source {
            VolumeSource::MountPoint(p) => Some(p),
            _ => None,
        }
    }

    /// Returns the Unix device path when opened directly.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[must_use]
    pub fn device_path(&self) -> Option<&Path> {
        match &self.source {
            VolumeSource::DevicePath(path) => Some(path),
            _ => None,
        }
    }

    /// Creates a mock `Volume` for testing (invalid handle, no real device).
    #[cfg(all(test, windows))]
    pub(crate) fn mock(handle: HANDLE, source: VolumeSource) -> Self {
        Volume { handle, source }
    }
}

#[cfg(windows)]
impl Drop for Volume {
    fn drop(&mut self) {
        if self.handle.is_invalid() {
            return;
        }

        // SAFETY: `self.handle` was returned by a successful open or duplication
        // call (see `from_drive_letter` / `from_mount_point`) and has not
        // been closed elsewhere. `Drop` runs at most once per `Volume`,
        // so this is the unique close. `CloseHandle` is safe to call on
        // a valid handle that the caller owns.
        if let Err(err) = unsafe { CloseHandle(self.handle) } {
            warn!("Failed to close volume handle: {err}");
        }
    }
}

/// Opens a handle to an NTFS/ReFS volume using a drive letter.
#[cfg(windows)]
fn get_volume_handle_from_drive_letter(drive_letter: char) -> Result<HANDLE, MftError> {
    if !privilege::is_elevated()? {
        return Err(MftError::NotElevated);
    }

    // https://learn.microsoft.com/en-us/windows/win32/fileio/obtaining-a-volume-handle-for-change-journal-operations
    // To obtain a handle to a volume for use with update sequence number (USN) change journal operations,
    // call the CreateFile function with the lpFileName parameter set to a string of the following form: \\.\X:
    // Note that X is the letter that identifies the drive on which the NTFS volume appears.
    let volume_root = format!(r"\\.\{drive_letter}:");

    // SAFETY: All pointer/reference parameters point to valid local
    // values constructed just above. `CreateFileW` accepts a wide-string
    // file name (provided through `HSTRING`) and may return either a
    // valid handle or a Win32 error; both outcomes are handled below.
    match unsafe {
        CreateFileW(
            &HSTRING::from(&volume_root),
            FILE_GENERIC_READ.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_OVERLAPPED,
            None,
        )
    } {
        Ok(handle) => Ok(handle),
        Err(err) if err == ERROR_ACCESS_DENIED.into() => Err(MftError::NotElevated),
        Err(err) => Err(MftError::WinApi(err)),
    }
}

/// Opens a handle to an NTFS/ReFS volume using a mount point path.
#[cfg(windows)]
fn get_volume_handle_from_mount_point(mount_point: &Path) -> Result<HANDLE, MftError> {
    if !privilege::is_elevated()? {
        return Err(MftError::NotElevated);
    }

    // GetVolumeNameForVolumeMountPointW requires trailing backslash
    let mount_path = format!("{}\\", mount_point.to_string_lossy());

    let mut volume_name = [0u16; 50]; // Enough space for volume GUID path
    // SAFETY: `mount_path` lives until the end of the call; `volume_name`
    // is a stack buffer of u16 we are writing into. The Win32 contract
    // for `GetVolumeNameForVolumeMountPointW` only requires the buffer
    // be large enough to hold the volume GUID path (50 wide chars is
    // ample for the standard `\\?\Volume{GUID}\` form).
    if let Err(err) =
        unsafe { GetVolumeNameForVolumeMountPointW(&HSTRING::from(&mount_path), &mut volume_name) }
    {
        warn!("GetVolumeNameForVolumeMountPointW failed, mount_point={mount_path}, error={err:?}");
        return Err(err.into());
    }

    // Convert the null-terminated wide string to a Rust string
    let end = volume_name
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(volume_name.len());
    let name_data = volume_name
        .get(..end)
        .ok_or_else(|| MftError::InvalidMountPoint("Failed to get volume name data".into()))?;
    let volume_guid = String::from_utf16_lossy(name_data);

    debug!("Volume GUID: {volume_guid}");

    // IMPORTANT: Remove the trailing backslash for CreateFileW
    let volume_path = volume_guid.trim_end_matches('\\').to_string();
    debug!("Using volume path: {volume_path}");

    // SAFETY: `volume_path` is a valid null-terminated wide string for
    // the duration of the call (held by the `HSTRING`). All other pointer
    // parameters are either `None` or owned defaults. The returned handle
    // (or error) is propagated to the caller, who becomes the owner.
    let volume_handle = unsafe {
        CreateFileW(
            &HSTRING::from(&volume_path),
            FILE_GENERIC_READ.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_OVERLAPPED,
            None,
        )?
    };

    Ok(volume_handle)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn open_read_only(path: &Path) -> Result<File, MftError> {
    #[cfg(target_os = "macos")]
    let raw_path = macos::raw_device_path(path);
    #[cfg(target_os = "macos")]
    let path = raw_path.as_deref().unwrap_or(path);
    OpenOptions::new()
        .read(true)
        .open(path)
        .map_err(MftError::Io)
}

#[cfg(target_os = "linux")]
fn device_for_mount_point(mount_point: &Path) -> Result<PathBuf, MftError> {
    let file = File::open("/proc/self/mountinfo")?;
    for line in BufReader::new(file).lines() {
        let line = line?;
        let Some((before, after)) = line.split_once(" - ") else {
            continue;
        };
        let fields: Vec<&str> = before.split_whitespace().collect();
        let after_fields: Vec<&str> = after.split_whitespace().collect();
        if fields.len() < 5 || after_fields.len() < 2 {
            continue;
        }
        let candidate = PathBuf::from(unescape_mountinfo(fields[4])?);
        if candidate != mount_point {
            continue;
        }
        if !matches!(after_fields[0], "ntfs" | "ntfs3" | "fuseblk") {
            return Err(MftError::UnsupportedFilesystem(
                "Linux mount point is not backed by NTFS",
            ));
        }
        let source = unescape_mountinfo(after_fields[1])?;
        if !source.starts_with('/') {
            return Err(MftError::InvalidMountPoint(
                "mount source is not a device path".into(),
            ));
        }
        return Ok(PathBuf::from(source));
    }
    Err(MftError::InvalidMountPoint(format!(
        "mount point not found in /proc/self/mountinfo: {}",
        mount_point.display()
    )))
}

#[cfg(target_os = "linux")]
fn unescape_mountinfo(value: &str) -> Result<String, MftError> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\\' {
            let digits = bytes
                .get(index + 1..index + 4)
                .ok_or_else(|| MftError::InvalidMountPoint("truncated mountinfo escape".into()))?;
            if !digits.iter().all(u8::is_ascii_digit) {
                return Err(MftError::InvalidMountPoint(
                    "invalid mountinfo escape".into(),
                ));
            }
            let text = std::str::from_utf8(digits)
                .map_err(|_| MftError::InvalidMountPoint("invalid mountinfo escape".into()))?;
            let decoded = u8::from_str_radix(text, 8)
                .map_err(|_| MftError::InvalidMountPoint("invalid mountinfo escape".into()))?;
            out.push(decoded);
            index += 4;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(out)
        .map_err(|_| MftError::InvalidMountPoint("mountinfo path is not UTF-8".into()))
}

#[cfg(all(test, target_os = "linux"))]
mod linux_tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn mountinfo_escapes_are_decoded() {
        assert_eq!(unescape_mountinfo(r"/media/a\040b").unwrap(), "/media/a b");
        assert!(unescape_mountinfo(r"/media/bad\0").is_err());
    }

    #[test]
    fn device_source_is_opened_read_only() {
        let volume = Volume::from_device_path("/dev/zero").unwrap();
        let fd = std::os::fd::AsRawFd::as_raw_fd(&volume.file);
        let mut fdinfo = String::new();
        File::open(format!("/proc/self/fdinfo/{fd}"))
            .unwrap()
            .read_to_string(&mut fdinfo)
            .unwrap();
        let flags = fdinfo
            .lines()
            .find_map(|line| line.strip_prefix("flags:\t"))
            .unwrap();
        let flags = u32::from_str_radix(flags, 8).unwrap();
        assert_eq!(flags & 0b11, 0, "descriptor must be O_RDONLY");
        assert_eq!(volume.device_path(), Some(Path::new("/dev/zero")));
    }
}

#[cfg(all(test, windows))]
mod tests {
    use windows::Win32::Foundation::HANDLE;

    use crate::volume::{Volume, VolumeSource};

    #[test]
    fn volume_source_accessors_construct_without_io() {
        let volume = Volume::mock(HANDLE(std::ptr::null_mut()), VolumeSource::DriveLetter('C'));
        assert_eq!(volume.drive_letter(), Some('C'));
    }
}

/// Scan-local shared read source. Duplicates refer to the same kernel file object;
/// Readers use explicit offsets and wait for their own completion event before
/// reusing a buffer. They never depend on the shared file cursor.
#[cfg(windows)]
#[derive(Clone)]
pub(crate) enum SharedVolumeSource {
    #[cfg(test)]
    Fixture(std::sync::Arc<sparse_fixture::SparseVolumeFixture>),
    Handle {
        handle: std::sync::Arc<std::os::windows::io::OwnedHandle>,
        source: VolumeSource,
    },
}

#[cfg(windows)]
impl Volume {
    /// Duplicate ownership without reopening the volume or repeating its first-read setup.
    pub(crate) fn shared_read_source(&self) -> Result<SharedVolumeSource, MftError> {
        use std::os::windows::io::BorrowedHandle;
        #[cfg(test)]
        if let Some(source) = self.sparse_fixture() {
            return Ok(SharedVolumeSource::Fixture(std::sync::Arc::clone(source)));
        }
        // SAFETY: this borrow is bounded by &self and immediately duplicated.
        let handle = unsafe { BorrowedHandle::borrow_raw(self.handle.0) }.try_clone_to_owned()?;
        Ok(SharedVolumeSource::Handle {
            handle: std::sync::Arc::new(handle),
            source: self.source.clone(),
        })
    }
}

#[cfg(windows)]
impl SharedVolumeSource {
    /// Give one worker its own RAII handle referring to the same file object.
    pub(crate) fn open(&self) -> Result<Volume, MftError> {
        use std::os::windows::io::IntoRawHandle;
        match self {
            #[cfg(test)]
            Self::Fixture(source) => source.open(),
            Self::Handle { handle, source } => Ok(Volume {
                handle: HANDLE(handle.try_clone()?.into_raw_handle()),
                source: source.clone(),
            }),
        }
    }
}
