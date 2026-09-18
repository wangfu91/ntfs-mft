//! Test live-device selection without opening any device or changing process state.
#[path = "../support/live_volume.rs"]
mod live_volume;

#[test]
fn live_source_requires_explicit_nonempty_configuration() {
    for source in [None, Some(std::ffi::OsString::new())] {
        let error = live_volume::required_source(source, "TEST_SOURCE").unwrap_err();
        assert!(
            matches!(error, ntfs_mft::MftError::Io(ref e) if e.kind() == std::io::ErrorKind::InvalidInput)
        );
        assert!(error.to_string().contains("TEST_SOURCE"));
    }
    let source = std::ffi::OsString::from("/dev/test-device");
    assert_eq!(
        live_volume::required_source(Some(source.clone()), "TEST_SOURCE").unwrap(),
        source
    );
}

#[test]
#[cfg(windows)]
fn drive_selection_rejects_paths_and_trailing_characters() {
    for source in ["c", "C:"] {
        assert_eq!(
            live_volume::parse_drive_letter(source.as_ref()).unwrap(),
            'C'
        );
    }
    for source in ["", "CD", "C:/", "C:\\", "1", " C", "é"] {
        assert!(
            live_volume::parse_drive_letter(source.as_ref()).is_err(),
            "{source}"
        );
    }
}
