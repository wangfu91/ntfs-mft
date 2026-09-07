//! Platform-independent NTFS name storage.
use std::{ffi::OsString, fmt};

/// An NTFS name retaining its exact UTF-16 code units on every platform.
///
/// Unpaired surrogates are preserved. Equality, ordering, and hashing compare
/// code units, with no case folding or Unicode normalization. A name is not
/// necessarily a safe path component; this type does not validate path syntax.
/// Use [`Self::as_utf16`] for identity/export and [`Self::to_string_lossy`] for
/// display. Native path conversion is explicit via [`Self::to_os_string`].
#[derive(Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NtfsName(Box<[u16]>);

impl NtfsName {
    /// An empty name.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Retain code units unchanged, including unpaired surrogates.
    #[must_use]
    pub fn from_utf16(units: impl Into<Box<[u16]>>) -> Self {
        Self(units.into())
    }

    /// Original UTF-16 code units.
    #[must_use]
    pub fn as_utf16(&self) -> &[u16] {
        &self.0
    }

    /// Recover the original UTF-16 storage without copying.
    #[must_use]
    pub fn into_utf16(self) -> Box<[u16]> {
        self.0
    }

    /// Whether the name contains no code units.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Convert to Unicode, rejecting unpaired surrogates.
    pub fn try_to_string(&self) -> Result<String, std::string::FromUtf16Error> {
        String::from_utf16(&self.0)
    }

    /// Display text, replacing each unpaired surrogate with U+FFFD.
    /// Distinct names can produce the same display text.
    #[must_use]
    pub fn to_string_lossy(&self) -> String {
        String::from_utf16_lossy(&self.0)
    }

    /// Convert to a native OS string.
    ///
    /// Windows preserves every code unit. Linux and macOS replace unpaired
    /// surrogates with U+FFFD; retain this `NtfsName` for lossless identity.
    #[must_use]
    pub fn to_os_string(&self) -> OsString {
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStringExt;
            OsString::from_wide(&self.0)
        }
        #[cfg(not(windows))]
        {
            OsString::from(self.to_string_lossy())
        }
    }
}

impl From<&str> for NtfsName {
    fn from(value: &str) -> Self {
        Self::from_utf16(value.encode_utf16().collect::<Vec<_>>())
    }
}
impl From<String> for NtfsName {
    fn from(value: String) -> Self {
        Self::from(value.as_str())
    }
}
impl PartialEq<str> for NtfsName {
    fn eq(&self, other: &str) -> bool {
        self.0.iter().copied().eq(other.encode_utf16())
    }
}
impl PartialEq<&str> for NtfsName {
    fn eq(&self, other: &&str) -> bool {
        self == *other
    }
}
impl fmt::Display for NtfsName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_string_lossy())
    }
}
impl fmt::Debug for NtfsName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NtfsName").field("utf16", &self.0).finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_units_distinguish_names_with_identical_display() {
        let a = NtfsName::from_utf16(vec![0x61, 0xd800]);
        let b = NtfsName::from_utf16(vec![0x61, 0xd801]);
        assert_ne!(a, b);
        assert_eq!(a.to_string_lossy(), b.to_string_lossy());
        assert!(a.try_to_string().is_err());
        assert_ne!(a, "a\u{fffd}");
        assert_eq!(a.clone().into_utf16().as_ref(), &[0x61, 0xd800]);
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            assert_eq!(
                a.to_os_string().encode_wide().collect::<Vec<_>>(),
                a.as_utf16()
            );
        }
        #[cfg(not(windows))]
        assert_eq!(a.to_os_string(), "a\u{fffd}");
    }
    #[test]
    fn unicode_round_trips_without_normalization() {
        let name = NtfsName::from("aé💫");
        assert_eq!(name.try_to_string().unwrap(), "aé💫");
        assert_eq!(name, "aé💫");
        assert_eq!(name.as_utf16(), &[0x61, 0xe9, 0xd83d, 0xdcab]);
        assert_ne!(NtfsName::from("é"), NtfsName::from("e\u{301}"));
        assert!(NtfsName::new().is_empty());
    }
}
