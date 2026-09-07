//! Strong newtype wrappers around the integer identifiers used by the
//! USN journal and the NTFS Master File Table.
//!
//! These types are `#[repr(transparent)]` over their underlying integer
//! representation, so wrapping or unwrapping is zero-cost.

use std::fmt;

/// Update Sequence Number — a monotonically increasing 64-bit signed
/// cursor into the USN change journal.
#[must_use]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(transparent)]
pub struct Usn(pub i64);

impl Usn {
    /// The zero USN — the conventional starting cursor for a full journal or
    /// MFT scan.
    pub const ZERO: Self = Self(0);

    /// Construct a `Usn` from its raw signed integer representation.
    #[inline]
    pub const fn new(v: i64) -> Self {
        Self(v)
    }

    /// Return the raw signed integer representation.
    #[inline]
    pub const fn get(self) -> i64 {
        self.0
    }

    /// Returns `true` if this is the zero USN (the conventional scan start).
    #[must_use]
    #[inline]
    pub const fn is_zero(self) -> bool {
        self.0 == 0
    }

    /// Advance this USN by `delta`, saturating at the numeric bounds.
    ///
    /// USNs are byte offsets into the change journal, so advancing by a byte
    /// delta is occasionally useful when resuming a scan.
    #[inline]
    pub const fn saturating_add(self, delta: i64) -> Self {
        Self(self.0.saturating_add(delta))
    }

    /// Advance this USN by `delta`, returning `None` on overflow.
    #[inline]
    pub const fn checked_add(self, delta: i64) -> Option<Self> {
        match self.0.checked_add(delta) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }
}

impl From<i64> for Usn {
    #[inline]
    fn from(v: i64) -> Self {
        Self(v)
    }
}

impl From<Usn> for i64 {
    #[inline]
    fn from(v: Usn) -> Self {
        v.0
    }
}

impl fmt::Display for Usn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

bitflags::bitflags! {
    /// Strongly-typed view over an NTFS file-attribute bitmask
    /// (the value stored in `USN_RECORD_V2::FileAttributes`,
    /// `USN_RECORD_V3::FileAttributes`,
    /// `MftEntry::file_attributes`, and `RawMftEntry::si_file_attributes`).
    ///
    /// Mirrors the Win32 `FILE_ATTRIBUTE_*` constants. Unknown bits are
    /// preserved on round-trip via [`bitflags`]'s `from_bits_retain`.
    #[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
    #[repr(transparent)]
    pub struct FileAttributes: u32 {
        /// Item is read-only.
        const READ_ONLY            = 0x0000_0001;
        /// Item is hidden from normal directory listings.
        const HIDDEN               = 0x0000_0002;
        /// Item is used by the operating system.
        const SYSTEM               = 0x0000_0004;
        /// Item is a directory.
        const DIRECTORY            = 0x0000_0010;
        /// Item should be archived.
        const ARCHIVE              = 0x0000_0020;
        /// Reserved device attribute.
        const DEVICE               = 0x0000_0040;
        /// Item has no other special attributes set.
        const NORMAL               = 0x0000_0080;
        /// Item should preferably be kept in temporary storage.
        const TEMPORARY            = 0x0000_0100;
        /// Item contains sparse data.
        const SPARSE_FILE          = 0x0000_0200;
        /// Item is represented by a reparse point.
        const REPARSE_POINT        = 0x0000_0400;
        /// Item is compressed on disk.
        const COMPRESSED           = 0x0000_0800;
        /// Item's data is not immediately available.
        const OFFLINE              = 0x0000_1000;
        /// Item should not be content indexed.
        const NOT_CONTENT_INDEXED  = 0x0000_2000;
        /// Item is encrypted on disk.
        const ENCRYPTED            = 0x0000_4000;
        /// Item uses integrity streams.
        const INTEGRITY_STREAM     = 0x0000_8000;
        /// Reserved virtual attribute.
        const VIRTUAL              = 0x0001_0000;
        /// Item is excluded from scrubber processing.
        const NO_SCRUB_DATA        = 0x0002_0000;
        /// Item is recalled when opened.
        const RECALL_ON_OPEN       = 0x0004_0000;
        /// Item is recalled on data access.
        const RECALL_ON_DATA_ACCESS = 0x0040_0000;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usn_round_trip() {
        let v = Usn::new(0x1234_5678);
        assert_eq!(v.get(), 0x1234_5678);
        let i: i64 = v.into();
        assert_eq!(i, 0x1234_5678);
        assert_eq!(Usn::from(0x42i64).get(), 0x42);
    }

    #[test]
    fn usn_display_is_decimal() {
        assert_eq!(format!("{}", Usn::new(0x10)), "16");
    }

    #[test]
    fn usn_is_zero_and_arithmetic() {
        assert!(Usn::ZERO.is_zero());
        assert!(!Usn::new(1).is_zero());

        assert_eq!(Usn::new(10).saturating_add(5), Usn::new(15));
        assert_eq!(Usn::new(i64::MAX).saturating_add(1), Usn::new(i64::MAX));

        assert_eq!(Usn::new(10).checked_add(5), Some(Usn::new(15)));
        assert_eq!(Usn::new(i64::MAX).checked_add(1), None);
    }

    #[test]
    fn file_attributes_display_is_none_for_zero_bits() {
        assert_eq!(format!("{}", FileAttributes::empty()), "NONE");
    }

    #[test]
    fn file_attributes_display_uses_hex_for_unknown_bits() {
        let attrs = FileAttributes::from_bits_retain(0x8000_0000);
        assert_eq!(format!("{attrs}"), "0x80000000");
    }

    #[test]
    fn usn_zero_const_is_zero() {
        assert_eq!(Usn::ZERO, Usn::new(0));
        assert_eq!(Usn::ZERO.get(), 0);
    }

    #[test]
    fn file_attributes_predicates() {
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
    fn file_attributes_display_lists_known_flags() {
        let attrs = FileAttributes::DIRECTORY | FileAttributes::HIDDEN;
        assert_eq!(attrs.to_string(), "HIDDEN | DIRECTORY");
    }
}

/// Failure to construct an NTFS-specific file reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum FileReferenceError {
    /// The record number cannot fit in NTFS's 48-bit slot field.
    #[error("record number {0} exceeds 48 bits")]
    RecordNumberOutOfRange(u64),
}

/// NTFS 64-bit file reference, including the 48-bit slot and 16-bit sequence.
///
/// This type cannot represent ReFS identifiers, so
/// [`Self::record_number`] and [`Self::sequence`] return values directly.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct NtfsFileReference(u64);

impl NtfsFileReference {
    /// Interpret the raw 64-bit NTFS reference without discarding any bits.
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }
    /// Construct a reference, rejecting record-number overflow instead of masking.
    pub const fn try_from_parts(
        record_number: u64,
        sequence: u16,
    ) -> Result<Self, FileReferenceError> {
        if record_number > 0x0000_ffff_ffff_ffff {
            Err(FileReferenceError::RecordNumberOutOfRange(record_number))
        } else {
            Ok(Self(((sequence as u64) << 48) | record_number))
        }
    }
    /// Raw packed file reference.
    pub const fn get(self) -> u64 {
        self.0
    }
    /// Record number, excluding the sequence field.
    pub const fn record_number(self) -> u64 {
        self.0 & 0x0000_ffff_ffff_ffff
    }
    /// Sequence used to distinguish generations of a reused record slot.
    pub const fn sequence(self) -> u16 {
        (self.0 >> 48) as u16
    }
    /// Raw little-endian bytes.
    pub const fn to_le_bytes(self) -> [u8; 8] {
        self.0.to_le_bytes()
    }
}
impl From<u64> for NtfsFileReference {
    fn from(raw: u64) -> Self {
        Self::new(raw)
    }
}
impl From<NtfsFileReference> for u64 {
    fn from(reference: NtfsFileReference) -> Self {
        reference.get()
    }
}
impl fmt::Display for NtfsFileReference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "0x{:x}", self.0)
    }
}
impl fmt::LowerHex for NtfsFileReference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::LowerHex::fmt(&self.0, f)
    }
}
impl fmt::UpperHex for NtfsFileReference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::UpperHex::fmt(&self.0, f)
    }
}

#[cfg(test)]
mod ntfs_reference_tests {
    use super::*;
    #[test]
    fn references_preserve_sequence_and_reject_overflow() {
        let reference = NtfsFileReference::try_from_parts(42, 7).unwrap();
        assert_eq!(reference.record_number(), 42);
        assert_eq!(reference.sequence(), 7);
        assert_eq!(format!("{reference}"), "0x700000000002a");
        assert_eq!(format!("{reference:#X}"), "0x700000000002A");
        assert_eq!(u64::from(reference), reference.get());
        assert_eq!(reference.get(), (7u64 << 48) | 42);
        assert_eq!(
            NtfsFileReference::new(u64::from_le_bytes(reference.to_le_bytes())),
            reference
        );
        assert!(NtfsFileReference::try_from_parts(1u64 << 48, 0).is_err());
        assert!(NtfsFileReference::try_from_parts(u64::MAX, 0).is_err());
        assert_eq!(
            NtfsFileReference::try_from_parts((1u64 << 48) - 1, u16::MAX)
                .unwrap()
                .get(),
            u64::MAX
        );
    }
}
