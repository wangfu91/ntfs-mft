//! Platform-independent NTFS name storage.
use std::{ffi::OsString, fmt};

/// A lossless NTFS name, stored as its on-disk UTF-16 code units.
///
/// NTFS names can contain unpaired UTF-16 surrogates, which cannot be
/// represented by a Rust [`String`]. `NtfsName` retains those units unchanged
/// on every host platform. Equality, ordering, and hashing compare the original
/// code units; they do not apply NTFS case folding or Unicode normalization.
///
/// Use [`Self::as_utf16`] when identity or lossless export matters.
/// [`Self::try_to_string`] and [`Self::to_string_lossy`] are text conversions
/// for consumers that require Unicode, while [`Self::to_os_string`] is an
/// explicit, platform-dependent conversion for native-path APIs. This type does
/// not validate whether a name is a safe path component.
///
/// # Examples
///
/// ```
/// use ntfs_mft::NtfsName;
///
/// let name = NtfsName::from_utf16(vec![b'a' as u16, 0xd800]);
/// assert_eq!(name.as_utf16(), &[0x0061, 0xd800]);
/// assert!(name.try_to_string().is_err());
/// assert_eq!(name.to_string_lossy(), "a\u{fffd}");
/// ```
#[derive(Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NtfsName(Box<[u16]>);

impl NtfsName {
    /// An empty name.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct a name from UTF-16 code units without validation or conversion.
    ///
    /// This preserves unpaired surrogates and does not apply case folding or
    /// Unicode normalization.
    #[must_use]
    pub fn from_utf16(units: impl Into<Box<[u16]>>) -> Self {
        Self(units.into())
    }

    /// Borrow the original UTF-16 code units.
    #[must_use]
    pub fn as_utf16(&self) -> &[u16] {
        &self.0
    }

    /// Return the original UTF-16 storage without copying.
    #[must_use]
    pub fn into_utf16(self) -> Box<[u16]> {
        self.0
    }

    /// Whether the name contains no code units.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Convert to a Unicode [`String`].
    ///
    /// Returns an error when the name contains an unpaired surrogate. Use
    /// [`Self::to_string_lossy`] for readable replacement text instead.
    pub fn try_to_string(&self) -> Result<String, std::string::FromUtf16Error> {
        String::from_utf16(&self.0)
    }

    /// Convert to display text, replacing unpaired surrogates with U+FFFD.
    ///
    /// This conversion is not suitable for identity or deduplication: distinct
    /// NTFS names can produce the same result.
    #[must_use]
    pub fn to_string_lossy(&self) -> String {
        String::from_utf16_lossy(&self.0)
    }

    /// Convert to a native OS string.
    ///
    /// On Windows, the returned [`OsString`] preserves every code unit. On
    /// Linux and macOS, unpaired surrogates become U+FFFD, so retain this
    /// `NtfsName` when identity or lossless export matters. The result is not
    /// validated as a safe path component.
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
/// Formats the lossy display representation from [`NtfsName::to_string_lossy`].
///
/// Do not use [`fmt::Display`] output as an identity key because distinct names
/// can display identically.
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
