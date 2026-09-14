//! Conversion helpers and the public `Filetime` wrapper.

use crate::errors::MftError;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
#[cfg(windows)]
use windows::Win32::Foundation::FILETIME;

/// Number of 100-nanosecond intervals between the Windows FILETIME epoch
/// (1601-01-01 UTC) and the Unix epoch (1970-01-01 UTC).
pub(crate) const WINDOWS_TO_UNIX_OFFSET_100NS: u64 = 116_444_736_000_000_000u64;

/// A Windows `FILETIME` value: a count of 100-nanosecond intervals since
/// 1601-01-01 UTC.
///
/// This is the raw timestamp representation used by the NTFS USN journal
/// and MFT records. It is exposed in this crate's public API in place of
/// any specific date/time type so that callers can pick whichever
/// downstream conversion suits their use case.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(transparent)]
pub struct Filetime(u64);

impl Filetime {
    /// The Unix epoch (1970-01-01 UTC) expressed as a `Filetime`.
    pub const UNIX_EPOCH: Self = Self(WINDOWS_TO_UNIX_OFFSET_100NS);

    /// Construct a `Filetime` from its raw 100-ns interval count.
    #[must_use]
    #[inline]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Return the raw 100-ns interval count since the Windows epoch.
    #[must_use]
    #[inline]
    pub const fn raw(self) -> u64 {
        self.0
    }

    /// Returns `true` if this is the zero `Filetime` (the value NTFS and the USN
    /// journal use for an unset timestamp).
    #[must_use]
    #[inline]
    pub const fn is_zero(self) -> bool {
        self.0 == 0
    }

    /// Convert to `SystemTime`.
    ///
    /// Returns `None` only when the resulting `SystemTime` cannot be
    /// represented on the current platform.
    #[must_use]
    pub fn to_system_time(self) -> Option<SystemTime> {
        if self.0 >= WINDOWS_TO_UNIX_OFFSET_100NS {
            let intervals = self.0 - WINDOWS_TO_UNIX_OFFSET_100NS;
            let secs = intervals / 10_000_000;
            let nanos = ((intervals % 10_000_000) * 100) as u32;
            UNIX_EPOCH.checked_add(Duration::new(secs, nanos))
        } else {
            let intervals = WINDOWS_TO_UNIX_OFFSET_100NS - self.0;
            let secs = intervals / 10_000_000;
            let nanos = ((intervals % 10_000_000) * 100) as u32;
            UNIX_EPOCH.checked_sub(Duration::new(secs, nanos))
        }
    }

    /// Convert from `SystemTime`.
    ///
    /// Returns `None` when the input is before the Windows FILETIME epoch or
    /// when the 100-nanosecond interval count would overflow `u64`.
    #[must_use]
    pub fn from_system_time(value: SystemTime) -> Option<Self> {
        system_time_to_filetime_raw(value).map(Self)
    }

    /// Number of seconds since the Unix epoch (may be negative).
    #[must_use]
    #[inline]
    pub fn to_unix_seconds(self) -> i64 {
        let intervals = self.0 as i128 - WINDOWS_TO_UNIX_OFFSET_100NS as i128;
        let seconds = intervals / 10_000_000;
        seconds.clamp(i64::MIN as i128, i64::MAX as i128) as i64
    }

    /// Number of milliseconds since the Unix epoch (may be negative).
    #[must_use]
    #[inline]
    pub fn to_unix_millis(self) -> i64 {
        let intervals = self.0 as i128 - WINDOWS_TO_UNIX_OFFSET_100NS as i128;
        let millis = intervals / 10_000;
        millis.clamp(i64::MIN as i128, i64::MAX as i128) as i64
    }

    /// Number of nanoseconds since the Unix epoch (may be negative).
    #[must_use]
    #[inline]
    pub fn to_unix_nanos(self) -> i128 {
        let intervals = self.0 as i128 - WINDOWS_TO_UNIX_OFFSET_100NS as i128;
        intervals * 100
    }

    /// Construct a `Filetime` from a whole number of seconds since the Unix epoch.
    ///
    /// Returns `None` when the result would fall before the Windows FILETIME
    /// epoch (1601-01-01) or overflow the underlying `u64`.
    ///
    /// # Examples
    ///
    /// ```
    /// use ntfs_mft::Filetime;
    ///
    /// let epoch = Filetime::from_unix_seconds(0).unwrap();
    /// assert_eq!(epoch, Filetime::UNIX_EPOCH);
    /// assert_eq!(epoch.to_unix_seconds(), 0);
    ///
    /// // Dates before 1601 are unrepresentable.
    /// assert!(Filetime::from_unix_seconds(-20_000_000_000).is_none());
    /// ```
    #[must_use]
    pub fn from_unix_seconds(seconds: i64) -> Option<Self> {
        let intervals = (seconds as i128).checked_mul(10_000_000)?;
        let raw = intervals.checked_add(WINDOWS_TO_UNIX_OFFSET_100NS as i128)?;
        if (0..=u64::MAX as i128).contains(&raw) {
            Some(Self(raw as u64))
        } else {
            None
        }
    }
}

#[cfg(windows)]
impl From<FILETIME> for Filetime {
    #[inline]
    fn from(value: FILETIME) -> Self {
        Self(((value.dwHighDateTime as u64) << 32) | value.dwLowDateTime as u64)
    }
}

#[cfg(windows)]
impl From<Filetime> for FILETIME {
    #[inline]
    fn from(value: Filetime) -> Self {
        Self {
            dwLowDateTime: value.raw() as u32,
            dwHighDateTime: (value.raw() >> 32) as u32,
        }
    }
}

impl TryFrom<SystemTime> for Filetime {
    type Error = MftError;

    fn try_from(value: SystemTime) -> Result<Self, Self::Error> {
        Self::from_system_time(value).ok_or(MftError::InvalidTimestamp(
            "SystemTime is outside the Windows FILETIME range",
        ))
    }
}

impl TryFrom<Filetime> for SystemTime {
    type Error = MftError;

    fn try_from(value: Filetime) -> Result<Self, Self::Error> {
        value.to_system_time().ok_or(MftError::InvalidTimestamp(
            "FILETIME is outside the SystemTime range",
        ))
    }
}

/// Convert a `SystemTime` into its raw Windows `FILETIME` representation.
fn system_time_to_filetime_raw(value: SystemTime) -> Option<u64> {
    match value.duration_since(UNIX_EPOCH) {
        Ok(duration) => {
            let intervals = duration.as_nanos() / 100;
            (WINDOWS_TO_UNIX_OFFSET_100NS as u128)
                .checked_add(intervals)
                .and_then(|raw| u64::try_from(raw).ok())
        }
        Err(err) => {
            let intervals = err.duration().as_nanos() / 100;
            if intervals > WINDOWS_TO_UNIX_OFFSET_100NS as u128 {
                None
            } else {
                Some(WINDOWS_TO_UNIX_OFFSET_100NS - intervals as u64)
            }
        }
    }
}

// These tests exercise the platform-independent contract of the public
// `Filetime` type. The Windows-only module below retains Win32 ABI and API
// checks; a non-Windows CI run must still cover the conversion arithmetic.
#[cfg(test)]
mod portable_tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn raw_value_and_zero_sentinel_are_preserved() {
        let raw = 0x0123_4567_89ab_cdef;
        let filetime = Filetime::new(raw);

        assert_eq!(filetime.raw(), raw);
        assert_eq!(Filetime::new(raw), filetime);
        assert!(Filetime::new(0).is_zero());
        assert!(!Filetime::UNIX_EPOCH.is_zero());
    }

    #[test]
    fn unix_epoch_and_signed_offsets_have_exact_units() {
        let before_epoch = Filetime::new(WINDOWS_TO_UNIX_OFFSET_100NS - 10_000_000);
        let after_epoch = Filetime::new(WINDOWS_TO_UNIX_OFFSET_100NS + 15_000_000);

        assert_eq!(Filetime::UNIX_EPOCH.raw(), WINDOWS_TO_UNIX_OFFSET_100NS);
        assert_eq!(Filetime::UNIX_EPOCH.to_unix_seconds(), 0);
        assert_eq!(Filetime::UNIX_EPOCH.to_unix_millis(), 0);
        assert_eq!(Filetime::UNIX_EPOCH.to_unix_nanos(), 0);
        assert_eq!(before_epoch.to_unix_seconds(), -1);
        assert_eq!(before_epoch.to_unix_millis(), -1_000);
        assert_eq!(before_epoch.to_unix_nanos(), -1_000_000_000);
        assert_eq!(after_epoch.to_unix_millis(), 1_500);
    }

    #[test]
    fn unix_epoch_round_trips_through_system_time() {
        let filetime = Filetime::from_system_time(UNIX_EPOCH).expect("Unix epoch");
        assert_eq!(filetime, Filetime::UNIX_EPOCH);
        assert_eq!(filetime.to_system_time(), Some(UNIX_EPOCH));

        let converted: SystemTime = filetime.try_into().expect("Unix epoch is representable");
        assert_eq!(converted, UNIX_EPOCH);
    }

    #[test]
    fn system_time_conversion_preserves_100ns_ticks_and_truncates_subticks() {
        let exact = UNIX_EPOCH + Duration::new(1, 123_456_700);
        let exact_filetime = Filetime::from_system_time(exact).expect("post-epoch time");
        assert_eq!(
            exact_filetime.raw(),
            WINDOWS_TO_UNIX_OFFSET_100NS + 10_000_000 + 1_234_567
        );
        assert_eq!(exact_filetime.to_system_time(), Some(exact));

        let sub_tick = UNIX_EPOCH + Duration::new(0, 150);
        let truncated = Filetime::from_system_time(sub_tick).expect("post-epoch time");
        assert_eq!(truncated.raw(), WINDOWS_TO_UNIX_OFFSET_100NS + 1);
        assert_eq!(
            truncated.to_system_time(),
            Some(UNIX_EPOCH + Duration::new(0, 100))
        );
    }

    #[test]
    fn known_post_epoch_value_round_trips_through_system_time() {
        // 2020-01-01T00:00:00Z, well inside every supported SystemTime range.
        let filetime = Filetime::new(132_223_104_000_000_000);
        assert_eq!(filetime.to_unix_seconds(), 1_577_836_800);
        assert_eq!(filetime.to_unix_nanos(), 1_577_836_800 * 1_000_000_000);

        let system_time = filetime.to_system_time().expect("2020 is representable");
        assert_eq!(Filetime::from_system_time(system_time), Some(filetime));
    }

    #[test]
    fn unix_second_construction_enforces_filetime_range() {
        let epoch = Filetime::from_unix_seconds(0).expect("Unix epoch");
        assert_eq!(epoch, Filetime::UNIX_EPOCH);

        let pre_epoch = Filetime::from_unix_seconds(-1).expect("1969 is after 1601");
        assert_eq!(pre_epoch.to_unix_seconds(), -1);

        assert!(Filetime::from_unix_seconds(-20_000_000_000).is_none());
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};
    use windows::Win32::{
        Foundation::{FILETIME, SYSTEMTIME as WinSystemTime},
        System::Time::SystemTimeToFileTime,
    };

    #[test]
    fn filetime_round_trips_through_win32_representation() {
        for raw in [0x0123_4567_89ab_cdef, u64::MAX] {
            let filetime = Filetime::new(raw);
            let win32: FILETIME = filetime.into();

            assert_eq!(Filetime::from(win32), filetime);
            assert_eq!(win32.dwLowDateTime, raw as u32);
            assert_eq!(win32.dwHighDateTime, (raw >> 32) as u32);
        }
    }

    #[test]
    fn windows_epoch_matches_system_time() {
        let expected = UNIX_EPOCH - Duration::from_secs(WINDOWS_TO_UNIX_OFFSET_100NS / 10_000_000);
        assert_eq!(Filetime::new(0).to_system_time(), Some(expected));
    }

    #[test]
    fn conversion_agrees_with_system_time_to_file_time() -> windows::core::Result<()> {
        let system_time = WinSystemTime {
            wYear: 2024,
            wMonth: 1,
            wDay: 1,
            wDayOfWeek: 0,
            wHour: 12,
            wMinute: 0,
            wSecond: 0,
            wMilliseconds: 0,
        };
        let mut win32_filetime = FILETIME::default();
        unsafe { SystemTimeToFileTime(&system_time, &mut win32_filetime)? };

        let filetime = Filetime::from(win32_filetime);
        let converted = filetime.to_system_time().expect("2024 is representable");
        assert_eq!(Filetime::from_system_time(converted), Some(filetime));
        Ok(())
    }
}
