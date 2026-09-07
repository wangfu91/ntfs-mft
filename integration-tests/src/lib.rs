//! Independent Windows parity-test support; never part of the published crate.

use ntfs_mft::{Fid, FileAttributes};

#[cfg(windows)]
pub mod windows;

#[derive(Debug)]
pub struct MftEntry {
    pub fid: Fid,
    pub parent_fid: Fid,
    pub file_name: std::ffi::OsString,
    pub file_attributes: FileAttributes,
}

impl MftEntry {
    pub fn is_dir(&self) -> bool {
        self.file_attributes.is_directory()
    }
}

/// Decode the variable-length Windows FSCTL record without aligned pointer casts.
/// Layout: https://learn.microsoft.com/en-us/windows/win32/api/winioctl/ns-winioctl-usn_record_v2
pub fn parse_record(bytes: &[u8]) -> std::io::Result<(MftEntry, usize)> {
    use std::io::{Error, ErrorKind};
    let invalid = || Error::new(ErrorKind::InvalidData, "invalid FSCTL enumeration record");
    if bytes.len() < 8 {
        return Err(invalid());
    }
    let length = u32::from_le_bytes(bytes[..4].try_into().unwrap()) as usize;
    let version = u16::from_le_bytes(bytes[4..6].try_into().unwrap());
    let (header, attrs, name_len, name_offset) = match version {
        2 => (60, 52, 56, 58),
        3 => (76, 68, 72, 74),
        _ => return Err(invalid()),
    };
    if length < header || length > bytes.len() || !length.is_multiple_of(8) {
        return Err(invalid());
    }
    let bytes = &bytes[..length];
    let len = u16::from_le_bytes(bytes[name_len..name_len + 2].try_into().unwrap()) as usize;
    let offset =
        u16::from_le_bytes(bytes[name_offset..name_offset + 2].try_into().unwrap()) as usize;
    if offset < header
        || !offset.is_multiple_of(2)
        || !len.is_multiple_of(2)
        || offset + len > length
    {
        return Err(invalid());
    }
    let (fid, parent_fid) = if version == 2 {
        (
            Fid::new(u64::from_le_bytes(bytes[8..16].try_into().unwrap())),
            Fid::new(u64::from_le_bytes(bytes[16..24].try_into().unwrap())),
        )
    } else {
        (
            Fid::from_bytes(bytes[8..24].try_into().unwrap()),
            Fid::from_bytes(bytes[24..40].try_into().unwrap()),
        )
    };
    let wide: Vec<u16> = bytes[offset..offset + len]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .collect();
    #[cfg(windows)]
    let file_name = {
        use std::os::windows::ffi::OsStringExt;
        std::ffi::OsString::from_wide(&wide)
    };
    #[cfg(not(windows))]
    let file_name = String::from_utf16_lossy(&wide).into();
    Ok((
        MftEntry {
            fid,
            parent_fid,
            file_name,
            file_attributes: FileAttributes::from_bits_retain(u32::from_le_bytes(
                bytes[attrs..attrs + 4].try_into().unwrap(),
            )),
        },
        length,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(version: u16) -> Vec<u8> {
        let (size, attrs, len, offset, header) = if version == 2 {
            (72, 52, 56, 58, 60)
        } else {
            (88, 68, 72, 74, 76)
        };
        let mut b = vec![0; size];
        b[..4].copy_from_slice(&(size as u32).to_le_bytes());
        b[4..6].copy_from_slice(&version.to_le_bytes());
        b[8..16].copy_from_slice(&0x1234000000000042u64.to_le_bytes());
        if version == 3 {
            b[16..24].copy_from_slice(&0xabcdefu64.to_le_bytes());
        }
        let parent = if version == 2 { 16 } else { 24 };
        b[parent..parent + 8].copy_from_slice(&5u64.to_le_bytes());
        b[attrs..attrs + 4].copy_from_slice(&0x10u32.to_le_bytes());
        b[len..len + 2].copy_from_slice(&4u16.to_le_bytes());
        b[offset..offset + 2].copy_from_slice(&(header as u16).to_le_bytes());
        b[header..header + 4].copy_from_slice(&[b'A', 0, 0x2d, 0x4e]);
        b
    }

    #[test]
    fn decodes_both_versions_and_preserves_ids_and_unicode() {
        for version in [2, 3] {
            let b = record(version);
            let (entry, consumed) = parse_record(&b).unwrap();
            assert_eq!(consumed, b.len());
            let expected_id =
                0x1234000000000042u128 | if version == 3 { 0xabcdefu128 << 64 } else { 0 };
            assert_eq!(entry.fid.as_u128(), expected_id);
            assert_eq!(entry.fid.is_standard(), version == 2);
            assert_eq!(entry.parent_fid.as_u128(), 5);
            assert_eq!(entry.file_name, "A中");
            assert!(entry.is_dir());
        }
    }

    #[test]
    fn rejects_truncation_invalid_lengths_versions_and_name_ranges() {
        for version in [2, 3] {
            let b = record(version);
            for end in 0..b.len() {
                assert!(parse_record(&b[..end]).is_err());
            }
            for (offset, value) in [
                (0, 0),
                (0, 255),
                (4, 4),
                (if version == 2 { 56 } else { 72 }, 255),
                (if version == 2 { 58 } else { 74 }, 0),
            ] {
                let mut bad = b.clone();
                bad[offset] = value;
                assert!(parse_record(&bad).is_err());
            }
        }
    }
}
