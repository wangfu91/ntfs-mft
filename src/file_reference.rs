//! NTFS Master File Table file references.
//!
//! This type is `#[repr(transparent)]` over its integer representation, so
//! wrapping or unwrapping is zero-cost.

use std::fmt;

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
    ) -> Result<Self, crate::FileReferenceError> {
        if record_number > 0x0000_ffff_ffff_ffff {
            Err(crate::FileReferenceError::RecordNumberOutOfRange(
                record_number,
            ))
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
mod tests {
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
