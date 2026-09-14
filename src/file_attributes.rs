//! NTFS file-attribute bitmasks and their formatting.

use std::fmt;

bitflags::bitflags! {
    /// Strongly-typed view over an NTFS file-attribute bitmask.
    ///
    /// Mirrors the Win32 `FILE_ATTRIBUTE_*` constants. Unknown bits are
    /// preserved on round-trip via [`bitflags`]'s `from_bits_retain`.
    #[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
    #[repr(transparent)]
    pub struct FileAttributes: u32 {
        /// Item is read-only.
        const READ_ONLY              = 0x0000_0001;
        /// Item is hidden from normal directory listings.
        const HIDDEN                 = 0x0000_0002;
        /// Item is used by the operating system.
        const SYSTEM                 = 0x0000_0004;
        /// Item is a directory.
        const DIRECTORY              = 0x0000_0010;
        /// Item should be archived.
        const ARCHIVE                = 0x0000_0020;
        /// Reserved device attribute.
        const DEVICE                 = 0x0000_0040;
        /// Item has no other special attributes set.
        const NORMAL                 = 0x0000_0080;
        /// Item should preferably be kept in temporary storage.
        const TEMPORARY              = 0x0000_0100;
        /// Item contains sparse data.
        const SPARSE_FILE            = 0x0000_0200;
        /// Item is represented by a reparse point.
        const REPARSE_POINT          = 0x0000_0400;
        /// Item is compressed on disk.
        const COMPRESSED             = 0x0000_0800;
        /// Item's data is not immediately available.
        const OFFLINE                = 0x0000_1000;
        /// Item should not be content indexed.
        const NOT_CONTENT_INDEXED    = 0x0000_2000;
        /// Item is encrypted on disk.
        const ENCRYPTED              = 0x0000_4000;
        /// Item uses integrity streams.
        const INTEGRITY_STREAM       = 0x0000_8000;
        /// Reserved virtual attribute.
        const VIRTUAL                = 0x0001_0000;
        /// Item is excluded from scrubber processing.
        const NO_SCRUB_DATA          = 0x0002_0000;
        /// Item is recalled when opened.
        const RECALL_ON_OPEN         = 0x0004_0000;
        /// Item is recalled on data access.
        const RECALL_ON_DATA_ACCESS  = 0x0040_0000;
    }
}

impl FileAttributes {
    /// Returns `true` if the directory attribute is set.
    #[must_use]
    #[inline]
    pub fn is_directory(self) -> bool {
        self.contains(Self::DIRECTORY)
    }

    /// Returns `true` if the read-only attribute is set.
    #[must_use]
    #[inline]
    pub fn is_read_only(self) -> bool {
        self.contains(Self::READ_ONLY)
    }

    /// Returns `true` if the hidden attribute is set.
    #[must_use]
    #[inline]
    pub fn is_hidden(self) -> bool {
        self.contains(Self::HIDDEN)
    }

    /// Returns `true` if the system attribute is set.
    #[must_use]
    #[inline]
    pub fn is_system(self) -> bool {
        self.contains(Self::SYSTEM)
    }

    /// Returns `true` if the archive attribute is set.
    #[must_use]
    #[inline]
    pub fn is_archive(self) -> bool {
        self.contains(Self::ARCHIVE)
    }

    /// Returns `true` if the item is a reparse point (symlink, mount point, etc.).
    #[must_use]
    #[inline]
    pub fn is_reparse_point(self) -> bool {
        self.contains(Self::REPARSE_POINT)
    }

    /// Returns `true` if the item is stored compressed on disk.
    #[must_use]
    #[inline]
    pub fn is_compressed(self) -> bool {
        self.contains(Self::COMPRESSED)
    }

    /// Returns `true` if the item is stored encrypted on disk.
    #[must_use]
    #[inline]
    pub fn is_encrypted(self) -> bool {
        self.contains(Self::ENCRYPTED)
    }

    /// Returns `true` if the item contains sparse data.
    #[must_use]
    #[inline]
    pub fn is_sparse(self) -> bool {
        self.contains(Self::SPARSE_FILE)
    }

    /// Returns `true` if the item's data is not immediately available (offline).
    #[must_use]
    #[inline]
    pub fn is_offline(self) -> bool {
        self.contains(Self::OFFLINE)
    }

    /// Returns `true` if the item is marked temporary.
    #[must_use]
    #[inline]
    pub fn is_temporary(self) -> bool {
        self.contains(Self::TEMPORARY)
    }
}

impl fmt::Display for FileAttributes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_empty() {
            f.write_str("NONE")
        } else {
            bitflags::parser::to_writer(self, f)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_is_none_for_zero_bits() {
        assert_eq!(format!("{}", FileAttributes::empty()), "NONE");
    }

    #[test]
    fn display_uses_hex_for_unknown_bits() {
        let attrs = FileAttributes::from_bits_retain(0x8000_0000);
        assert_eq!(format!("{attrs}"), "0x80000000");
    }

    #[test]
    fn display_preserves_unknown_bits_with_known_flags() {
        let attrs = FileAttributes::from_bits_retain(FileAttributes::HIDDEN.bits() | 0x8000_0000);
        assert_eq!(format!("{attrs}"), "HIDDEN | 0x80000000");
    }

    #[test]
    fn predicates() {
        assert!(FileAttributes::DIRECTORY.is_directory());
        assert!(FileAttributes::HIDDEN.is_hidden());
        assert!(FileAttributes::READ_ONLY.is_read_only());
        assert!(FileAttributes::SYSTEM.is_system());
        assert!(FileAttributes::ARCHIVE.is_archive());
        assert!(FileAttributes::REPARSE_POINT.is_reparse_point());
        assert!(FileAttributes::COMPRESSED.is_compressed());
        assert!(FileAttributes::ENCRYPTED.is_encrypted());
        assert!(FileAttributes::SPARSE_FILE.is_sparse());
        assert!(FileAttributes::OFFLINE.is_offline());
        assert!(FileAttributes::TEMPORARY.is_temporary());
        assert!(!FileAttributes::ARCHIVE.is_directory());

        let combined = FileAttributes::DIRECTORY | FileAttributes::HIDDEN;
        assert!(combined.is_directory());
        assert!(combined.is_hidden());
        assert!(!combined.is_read_only());
    }

    #[test]
    fn display_lists_known_flags() {
        let attrs = FileAttributes::DIRECTORY | FileAttributes::HIDDEN;
        assert_eq!(attrs.to_string(), "HIDDEN | DIRECTORY");
    }
}
