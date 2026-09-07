//! Internal path-resolution helpers.

use std::{
    cell::RefCell,
    ffi::{OsStr, OsString, c_void},
    mem::size_of,
    os::windows::ffi::OsStringExt,
    path::PathBuf,
};
use windows::Win32::{
    Foundation,
    Storage::FileSystem::{self, FILE_FLAGS_AND_ATTRIBUTES, FILE_ID_DESCRIPTOR},
};

use crate::{NtfsFileReference, volume::Volume};

/// Resolve a path without using the directory cache.
pub(crate) fn resolve_path(
    volume: &Volume,
    fid: NtfsFileReference,
    parent_fid: NtfsFileReference,
    file_name: &OsStr,
    buffer: &RefCell<Vec<u8>>,
) -> Option<PathBuf> {
    if let Ok(resolved_parent_path) = file_id_to_path(volume, parent_fid, buffer) {
        return Some(resolved_parent_path.join(file_name));
    } else if let Ok(resolved_path) = file_id_to_path(volume, fid, buffer) {
        return Some(resolved_path);
    }

    None
}

/// Resolves a file ID to its full path on the specified NTFS volume.
fn file_id_to_path(
    volume: &Volume,
    file_id: NtfsFileReference,
    buffer: &RefCell<Vec<u8>>,
) -> windows::core::Result<PathBuf> {
    let id = FileSystem::FILE_ID_DESCRIPTOR_0 {
        FileId: i64::from_ne_bytes(file_id.get().to_ne_bytes()),
    };
    let id_type = FileSystem::FileIdType;
    let file_id_desc = FILE_ID_DESCRIPTOR {
        Type: id_type,
        dwSize: size_of::<FileSystem::FILE_ID_DESCRIPTOR>() as u32,
        Anonymous: id,
    };

    // SAFETY: `volume.handle` is a live volume handle owned by `volume`.
    // `&file_id_desc` is a stack-local that outlives the call. Returns
    // either an owned file handle or an error; ownership transfers to us.
    let file_handle = unsafe {
        FileSystem::OpenFileById(
            volume.handle,
            &file_id_desc,
            0,
            FileSystem::FILE_SHARE_READ
                | FileSystem::FILE_SHARE_WRITE
                | FileSystem::FILE_SHARE_DELETE,
            None,
            FILE_FLAGS_AND_ATTRIBUTES(FileSystem::FILE_FLAG_BACKUP_SEMANTICS.0),
        )?
    };

    // Reuse the per-resolver buffer to avoid reallocating per call.
    let mut info_buffer = buffer.borrow_mut();
    let min_len = size_of::<FileSystem::FILE_NAME_INFO>() + 128 * size_of::<u16>();
    if info_buffer.len() < min_len {
        info_buffer.resize(min_len, 0);
    }

    loop {
        // SAFETY: `file_handle` is a live, owned handle from the
        // `OpenFileById` call above. `info_buffer` is a `Vec<u8>` of
        // exactly `info_buffer.len()` writable bytes; the FSCTL writes
        // a `FILE_NAME_INFO` into the front of that buffer.
        if let Err(err) = unsafe {
            FileSystem::GetFileInformationByHandleEx(
                file_handle,
                FileSystem::FileNameInfo,
                info_buffer.as_mut_ptr() as *mut c_void,
                info_buffer.len() as u32,
            )
        } {
            if err.code() == Foundation::ERROR_MORE_DATA.into() {
                // Long paths, needs to extend buffer size to hold it.
                let name_info = unsafe {
                    std::ptr::read(info_buffer.as_ptr() as *const FileSystem::FILE_NAME_INFO)
                };

                let needed_len = name_info.FileNameLength + size_of::<u32>() as u32;
                info_buffer.resize(needed_len as usize, 0);
                continue;
            }

            // SAFETY: `file_handle` is the live, owned handle from
            // `OpenFileById` above; we are closing it exactly once on
            // the error path. We deliberately ignore any close error
            // here because we are already returning the original `err`.
            unsafe {
                let _ = Foundation::CloseHandle(file_handle);
            };
            return Err(err);
        }

        break;
    }

    // SAFETY: `file_handle` is the live, owned handle from
    // `OpenFileById`; this is the unique close on the success path.
    unsafe { Foundation::CloseHandle(file_handle) }?;
    // SAFETY: The successful `GetFileInformationByHandleEx` call above
    // wrote a `FILE_NAME_INFO` (sized to fit) into the front of
    // `info_buffer`.
    let info: &FileSystem::FILE_NAME_INFO =
        unsafe { &*(info_buffer.as_ptr() as *const FileSystem::FILE_NAME_INFO) };

    let name_len = info.FileNameLength as usize / size_of::<u16>();
    // SAFETY: `info` was filled by a successful FSCTL call, so its
    // `FileNameLength` reflects the true number of UTF-16 bytes
    // written into the trailing `FileName` array.
    let name_u16 = unsafe { std::slice::from_raw_parts(info.FileName.as_ptr(), name_len) };
    let sub_path = OsString::from_wide(name_u16);

    let mut full_path = PathBuf::new();

    if let Some(drive_letter) = volume.drive_letter() {
        let drive_letter = if drive_letter.is_ascii_lowercase() {
            drive_letter.to_ascii_uppercase()
        } else {
            drive_letter
        };

        full_path.push(format!("{drive_letter}:\\"));
    } else if let Some(mount_point) = volume.mount_point() {
        full_path.push(mount_point);
    }

    full_path.push(sub_path);
    Ok(full_path)
}
