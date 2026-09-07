//! Native macOS mount lookup. No subprocesses or disk-management mutations.

use std::{
    ffi::{CString, OsString},
    fs::File,
    mem::MaybeUninit,
    os::{
        fd::AsRawFd,
        unix::ffi::{OsStrExt, OsStringExt},
    },
    path::{Path, PathBuf},
};

use crate::MftError;

/// Map only canonical BSD block-device names to their raw character devices.
pub(super) fn raw_device_path(path: &Path) -> Option<PathBuf> {
    if path.parent()? != Path::new("/dev") {
        return None;
    }
    let name = path.file_name()?.to_str()?;
    let suffix = name.strip_prefix("disk")?;
    if !suffix
        .split('s')
        .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    Some(Path::new("/dev").join(format!("r{name}")))
}

pub(super) fn device_alignment(file: &File) -> Result<u64, MftError> {
    if file.metadata()?.file_type().is_file() {
        return Ok(1);
    }
    // <sys/disk.h>: DKIOCGETBLOCKSIZE = _IOR('d', 24, uint32_t).
    const DKIOCGETBLOCKSIZE: libc::c_ulong = 0x4004_6418;
    let mut size = 0u32;
    // SAFETY: file owns a live descriptor, and size is writable storage of
    // exactly the uint32_t type required by this read-only device query.
    if unsafe { libc::ioctl(file.as_raw_fd(), DKIOCGETBLOCKSIZE, &mut size) } == -1 {
        return Err(std::io::Error::last_os_error().into());
    }
    if !size.is_power_of_two() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "invalid macOS device block size",
        )
        .into());
    }
    Ok(u64::from(size))
}

pub(super) fn device_for_mount_point(mount_point: &Path) -> Result<PathBuf, MftError> {
    let path = CString::new(mount_point.as_os_str().as_bytes())
        .map_err(|_| MftError::InvalidMountPoint("mount point contains NUL".into()))?;
    let mut info = MaybeUninit::<libc::statfs>::uninit();
    // SAFETY: path is NUL-terminated and info points to writable, correctly
    // aligned storage. statfs initializes the structure on success only.
    if unsafe { libc::statfs(path.as_ptr(), info.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    // SAFETY: the successful statfs call initialized info.
    let info = unsafe { info.assume_init() };
    device_from_mount_info(mount_point, &info)
}

fn mount_field(field: &[libc::c_char]) -> Result<OsString, MftError> {
    let end = field
        .iter()
        .position(|&byte| byte == 0)
        .ok_or_else(|| MftError::InvalidMountPoint("unterminated macOS mount metadata".into()))?;
    Ok(OsString::from_vec(
        field[..end].iter().map(|&byte| byte as u8).collect(),
    ))
}

fn device_from_mount_info(mount_point: &Path, info: &libc::statfs) -> Result<PathBuf, MftError> {
    let mounted_at = PathBuf::from(mount_field(&info.f_mntonname)?);
    // statfs accepts any child path. Match the exact mount root, as on Linux,
    // so a normal directory cannot silently select its containing device.
    if mounted_at != mount_point {
        return Err(MftError::InvalidMountPoint(format!(
            "not a volume mount point: {}",
            mount_point.display()
        )));
    }
    if mount_field(&info.f_fstypename)? != "ntfs" {
        return Err(MftError::UnsupportedFilesystem(
            "macOS mount point is not backed by the ntfs filesystem; use from_device_path for other NTFS drivers",
        ));
    }
    let device = PathBuf::from(mount_field(&info.f_mntfromname)?);
    if device.parent() != Some(Path::new("/dev")) {
        return Err(MftError::InvalidMountPoint(
            "macOS mount source is not a /dev device path".into(),
        ));
    }
    // open_read_only selects the raw character interface. XNU rejects opening
    // a mounted block device, including for read-only access.
    Ok(device)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_device_mapping_is_limited_to_bsd_disk_names() {
        for (input, expected) in [
            ("/dev/disk6s1", "/dev/rdisk6s1"),
            ("/dev/disk12", "/dev/rdisk12"),
            ("/dev/disk3s1s1", "/dev/rdisk3s1s1"),
        ] {
            assert_eq!(
                raw_device_path(Path::new(input)).as_deref(),
                Some(Path::new(expected))
            );
        }
        for input in [
            "/dev/rdisk6s1",
            "/tmp/disk6s1",
            "/dev/disk",
            "/dev/disk6s",
            "/dev/disk6oops",
            "/dev/zero",
        ] {
            assert!(raw_device_path(Path::new(input)).is_none());
        }
    }

    fn mount_info(mount: &[u8], kind: &[u8], source: &[u8]) -> libc::statfs {
        // SAFETY: statfs consists entirely of integer fields and arrays.
        let mut info: libc::statfs = unsafe { std::mem::zeroed() };
        for (field, value) in [
            (&mut info.f_mntonname[..], mount),
            (&mut info.f_fstypename[..], kind),
            (&mut info.f_mntfromname[..], source),
        ] {
            for (out, &byte) in field.iter_mut().zip(value) {
                *out = byte as libc::c_char;
            }
        }
        info
    }

    #[test]
    fn resolves_ntfs_mount_with_spaces_and_non_utf8_bytes() {
        let mount = b"/Volumes/NTFS test-\xff";
        let path = PathBuf::from(OsString::from_vec(mount.to_vec()));
        let info = mount_info(mount, b"ntfs", b"/dev/disk6s1");
        assert_eq!(
            device_from_mount_info(&path, &info).unwrap(),
            Path::new("/dev/disk6s1")
        );
    }

    #[test]
    fn rejects_child_directory_non_ntfs_and_non_device_sources() {
        let root = Path::new("/Volumes/test");
        let info = mount_info(b"/Volumes/test", b"ntfs", b"/dev/disk6s1");
        assert!(matches!(
            device_from_mount_info(&root.join("child"), &info),
            Err(MftError::InvalidMountPoint(_))
        ));
        let info = mount_info(b"/Volumes/test", b"apfs", b"/dev/disk3s1");
        assert!(matches!(
            device_from_mount_info(root, &info),
            Err(MftError::UnsupportedFilesystem(_))
        ));
        let info = mount_info(b"/Volumes/test", b"ntfs", b"server:/disk");
        assert!(matches!(
            device_from_mount_info(root, &info),
            Err(MftError::InvalidMountPoint(_))
        ));
        assert!(mount_field(&[b'x' as libc::c_char; 16]).is_err());
    }

    #[test]
    fn native_lookup_rejects_non_ntfs_root() {
        assert!(matches!(
            device_for_mount_point(Path::new("/")),
            Err(MftError::UnsupportedFilesystem(_))
        ));
    }

    #[test]
    fn device_descriptor_and_reopen_are_read_only() {
        use std::os::fd::AsRawFd;
        let path = std::env::current_exe().unwrap();
        let volume = crate::Volume::from_device_path(&path).unwrap();
        let reopened = volume.reopen().unwrap();
        for volume in [&volume, &reopened] {
            // SAFETY: the descriptor is live and F_GETFL takes no third argument.
            let flags = unsafe { libc::fcntl(volume.file.as_raw_fd(), libc::F_GETFL) };
            assert!(flags >= 0);
            assert_eq!(flags & libc::O_ACCMODE, libc::O_RDONLY);
            assert_eq!(volume.device_path(), Some(path.as_path()));
            assert_eq!(volume.io_alignment, 1);
            assert!(volume.mount_point().is_none());
            assert!(volume.drive_letter().is_none());
        }
    }
}
