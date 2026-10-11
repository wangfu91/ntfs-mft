//! Privilege helpers for Windows elevation checks.
//!
//! This module contains the small amount of Win32 token plumbing needed to
//! determine whether the current process is running elevated. The crate uses
//! this check to gate operations that require administrative access to a
//! volume or the USN journal.

use std::mem::size_of;

use windows::Win32::{
    Foundation::HANDLE,
    Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation},
    System::Threading::{GetCurrentProcess, OpenProcessToken},
};
use windows::core::Owned;

/// Returns whether the current process is running elevated.
///
/// This checks the process token via `OpenProcessToken` and
/// `GetTokenInformation(TokenElevation, ...)`.
///
/// # Errors
///
/// Returns the underlying Win32 error if opening the process token or reading
/// its elevation state fails.
pub fn is_elevated() -> windows::core::Result<bool> {
    is_elevated_with(open_process_token, token_is_elevated)
}

fn is_elevated_with<Token>(
    open_token: impl FnOnce() -> windows::core::Result<Token>,
    query_elevation: impl FnOnce(&Token) -> windows::core::Result<bool>,
) -> windows::core::Result<bool> {
    let token = open_token()?;
    query_elevation(&token)
}

fn open_process_token() -> windows::core::Result<Owned<HANDLE>> {
    let mut handle: HANDLE = HANDLE::default();
    // SAFETY: `GetCurrentProcess` returns a pseudo-handle with the
    // necessary access; `&mut handle` is a valid out-pointer. The Win32
    // call writes a real token handle on success which we wrap in
    // `Owned` immediately so it is closed on scope exit.
    unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut handle)? };
    // SAFETY: `handle` was initialised by the successful `OpenProcessToken`
    // call above and ownership has not been transferred elsewhere.
    Ok(unsafe { Owned::new(handle) })
}

fn token_is_elevated(handle: &Owned<HANDLE>) -> windows::core::Result<bool> {
    let mut elevation = TOKEN_ELEVATION::default();
    let mut returned_length = 0;

    // SAFETY: `**handle` is a live token handle borrowed from the
    // `Owned` wrapper; `&mut elevation` and `&mut returned_length` are
    // valid out-pointers, and the buffer length matches the size of
    // `TOKEN_ELEVATION` exactly.
    unsafe {
        GetTokenInformation(
            **handle,
            TokenElevation,
            Some(&mut elevation as *mut _ as *mut _),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned_length,
        )?
    };

    Ok(elevation.TokenIsElevated != 0)
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use windows::Win32::Foundation::{ERROR_ACCESS_DENIED, ERROR_INVALID_HANDLE};

    use super::*;

    struct MockToken<'a>(&'a Cell<usize>);

    impl Drop for MockToken<'_> {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }

    #[test]
    fn test_open_process_token_failure() {
        let queried = Cell::new(false);
        let result = is_elevated_with(
            || Err::<MockToken<'_>, _>(windows::core::Error::from(ERROR_ACCESS_DENIED)),
            |_| {
                queried.set(true);
                Ok(false)
            },
        );

        assert!(matches!(result, Err(error) if error.code() == ERROR_ACCESS_DENIED.into()));
        assert!(!queried.get());
    }

    #[test]
    fn test_get_token_information_failure_closes_token() {
        let drops = Cell::new(0);
        let result = is_elevated_with(
            || Ok(MockToken(&drops)),
            |token| {
                assert_eq!(token.0.get(), 0);
                Err(windows::core::Error::from(ERROR_INVALID_HANDLE))
            },
        );

        assert!(matches!(result, Err(error) if error.code() == ERROR_INVALID_HANDLE.into()));
        assert_eq!(drops.get(), 1);
    }

    #[test]
    fn test_get_token_information_success_closes_token() -> windows::core::Result<()> {
        for elevated in [false, true] {
            let drops = Cell::new(0);
            let result = is_elevated_with(
                || Ok(MockToken(&drops)),
                |token| {
                    assert_eq!(token.0.get(), 0);
                    Ok(elevated)
                },
            )?;

            assert_eq!(result, elevated);
            assert_eq!(drops.get(), 1);
        }
        Ok(())
    }
}
