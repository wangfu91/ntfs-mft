//! NTFS attribute decoding.
//!
//! This module is split into small focused submodules:
//! - `headers`: on-disk NTFS attribute layouts, enums, and flags
//! - `view`: `NtfsAttribute` plus typed accessors for resident payloads
//! - `iter`: record-slice iterators over attributes and `$ATTRIBUTE_LIST` entries

mod headers;
mod iter;
mod view;

pub use headers::FileNameNamespace;
pub(crate) use headers::{
    AttributeListEntryHeader, NtfsAttributeHeader, NtfsAttributeType, NtfsFileNameHeader,
    NtfsNonResidentAttributeHeader, NtfsResidentAttributeHeader, NtfsStandardInformation,
    attr_header_flags, file_attr_flags,
};
pub(crate) use iter::{
    AttributeListReference, for_each_attr_list_entry_header, for_each_attribute,
    for_each_attribute_checked, parse_attribute_list_references,
};
pub(crate) use view::{NtfsAttribute, ntfs_name_from_utf16le};
