use super::*;

mod differential;
mod fixture;
mod sparse_volume_fixture;

#[test]
fn options_defaults_are_sensible() {
    let o = RawMftScanOptions::default();
    assert_eq!(o.buffers().main(), DEFAULT_BUFFER_BYTES);
    assert_eq!(o.buffers().attr(), DEFAULT_ATTR_BUFFER_BYTES);
    assert!(!o.include_unused_records());
    assert!(o.entry().collect_alternate_data_streams());
    assert!(o.entry().collect_dos_file_name_links());
    assert_eq!(
        o.range().start_record(),
        super::layout::record::FIRST_NORMAL_RECORD
    );
    assert!(o.range().end_record().is_none());
}

#[test]
fn raw_mft_entry_display_is_compact() -> Result<(), crate::FileReferenceError> {
    use crate::{FileAttributes, Filetime, NtfsFileReference};

    let entry = RawMftEntry {
        observed_fields: 0,
        collection_options: crate::RawMftEntryOptions::default(),
        record_number: 42,
        sequence_number: 1,
        file_reference: NtfsFileReference::try_from_parts(42, 1)?,
        parent_reference: NtfsFileReference::try_from_parts(5, 1)?,
        base_reference: NtfsFileReference::new(0),
        hard_link_count: 2,
        flags: 0,
        is_used: true,
        is_directory: false,
        is_reparse_point: false,
        reparse_tag: None,
        namespace: FileNameNamespace::Win32,
        file_name: crate::NtfsName::from("file.txt"),
        si_created: Filetime::new(0),
        si_modified: Filetime::new(0),
        si_mft_modified: Filetime::new(0),
        si_accessed: Filetime::new(0),
        si_file_attributes: FileAttributes::empty(),
        fn_created: Filetime::new(0),
        fn_modified: Filetime::new(0),
        fn_mft_modified: Filetime::new(0),
        fn_accessed: Filetime::new(0),
        real_size: 1234,
        allocated_size: 4096,
        has_unnamed_data: true,
        is_resident: false,
        is_sparse: false,
        is_compressed: false,
        is_encrypted: false,
        alternate_data_streams: Box::default(),
        links: Box::default(),
    };

    let text = entry.to_string();
    assert!(text.contains("raw #42"));
    assert!(text.contains("FILE"));
    assert!(text.contains("links=2"));
    assert!(text.contains("size=1234"));
    assert!(text.contains("\"file.txt\""));
    Ok(())
}
