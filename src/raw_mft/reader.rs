//! Shared low-level read helpers reused by the raw-MFT iterator,
//! single-record access path, and `$ATTRIBUTE_LIST` enrichment code.

use std::io::{Read, Seek, SeekFrom};

use crate::{
    errors::MftError,
    raw_mft::{
        attr_list::{enrich_from_attr_list, should_enrich_from_attr_list},
        entry_build::{AttributeListInfo, EntryBuildOptions, RawMftEntry},
        layout::{boot::BootSector, data_run::DataRun, extent::ExtentMap, record::FileRecord},
        options::RawMftScanOptions,
        volume_reader::VolumeReader,
    },
};

use super::RawMft;

impl<'a> RawMft<'a> {
    /// Create the pair of buffered readers used by serial and chunked scans:
    /// one for the main sequential path and one for extension-record lookups.
    pub(super) fn buffered_readers_for_options(
        &self,
        options: &RawMftScanOptions,
    ) -> Result<(VolumeReader, VolumeReader), MftError> {
        let reader = self.volume_reader(options.buffers.main)?;
        let attr_reader = self.volume_reader(options.buffers.attr)?;
        Ok((reader, attr_reader))
    }
}

/// Convert iterator options into the entry-build options consumed by the
/// rich entry builder.
pub(super) fn entry_build_options(options: &RawMftScanOptions) -> EntryBuildOptions {
    EntryBuildOptions {
        collect_alternate_data_streams: options.entry.collect_alternate_data_streams,
        collect_dos_file_name_links: options.entry.collect_dos_file_name_links,
    }
}

/// Shared rich materialization for serial and parallel scans.
pub(super) fn build_rich_entry(
    mft: &RawMft<'_>,
    record: &FileRecord<'_>,
    attr_reader: &mut VolumeReader,
    build_options: EntryBuildOptions,
) -> RawMftEntry {
    let record_number = record.number;
    let (mut entry, attr_list) = RawMftEntry::from_record_with_attr_list(record, build_options);
    if let Some(attr_list) = attr_list
        && should_enrich_from_attr_list(&entry)
    {
        let _ = enrich_from_attr_list(
            &mut entry,
            attr_list,
            record_number,
            attr_reader,
            &mft.boot,
            mft.extent_map.as_ref(),
            build_options,
        );
    }
    entry
}

/// Look up, borrow, validate, and USA-fix-up one base FILE record.
///
/// Returns `Ok(None)` when the record number maps to a sparse hole in the
/// `$MFT` or the bytes are not a valid FILE record. Shared by the rich and
/// lean per-record readers below.
pub(super) fn parse_record_at<'b>(
    reader: &'b mut VolumeReader,
    extent_map: &ExtentMap,
    record_number: u64,
) -> Result<Option<FileRecord<'b>>, MftError> {
    let Some((offset, buf)) =
        reader.record_buffer(extent_map, record_number, &mut Default::default())?
    else {
        return Ok(None);
    };
    if !FileRecord::is_valid(buf) {
        return Ok(None);
    }
    Ok(Some(FileRecord::parse(record_number, Some(offset), buf)?))
}

/// Read one raw FILE record and return the rich entry plus any captured
/// `$ATTRIBUTE_LIST` payload.
pub(super) fn read_record_raw(
    reader: &mut VolumeReader,
    extent_map: &ExtentMap,
    record_number: u64,
    build_options: EntryBuildOptions,
) -> Result<Option<(RawMftEntry, Option<AttributeListInfo>)>, MftError> {
    let Some(rec) = parse_record_at(reader, extent_map, record_number)? else {
        return Ok(None);
    };
    Ok(Some(RawMftEntry::from_record_with_attr_list(
        &rec,
        build_options,
    )))
}

/// Read a single record and apply one level of `$ATTRIBUTE_LIST` enrichment
/// when the base entry needs data from extension records.
pub(super) fn read_record_at(
    reader: &mut VolumeReader,
    boot: &BootSector,
    extent_map: &ExtentMap,
    record_number: u64,
    build_options: EntryBuildOptions,
) -> Result<Option<RawMftEntry>, MftError> {
    let (mut entry, attr_list) =
        match read_record_raw(reader, extent_map, record_number, build_options)? {
            Some(tuple) => tuple,
            None => return Ok(None),
        };
    if let Some(attr_list) = attr_list
        && should_enrich_from_attr_list(&entry)
    {
        let _ = enrich_from_attr_list(
            &mut entry,
            attr_list,
            record_number,
            reader,
            boot,
            extent_map,
            build_options,
        );
    }
    Ok(Some(entry))
}

/// Materialize the contents of a non-resident attribute into memory by
/// reading its decoded data runs from disk.
/// Metadata payloads are bounded independently of their untrusted run lengths.
pub(super) fn read_nonresident(
    reader: &mut VolumeReader,
    runs: &[DataRun],
    cluster_size: u64,
    data_size: u64,
) -> Result<Vec<u8>, MftError> {
    const MAX_METADATA_BYTES: u64 = 64 * 1024 * 1024;
    if data_size > MAX_METADATA_BYTES {
        return Err(MftError::InvalidDataRun(
            "non-resident metadata exceeds the 64 MiB materialization limit",
        ));
    }
    // Validate the complete requested range before allocating or performing I/O.
    let mut uncovered = data_size;
    for run in runs {
        if uncovered == 0 {
            break;
        }
        let clusters = match *run {
            DataRun::Data { clusters, .. } | DataRun::Sparse { clusters } => clusters,
        };
        let bytes = clusters
            .checked_mul(cluster_size)
            .ok_or(MftError::InvalidDataRun("run byte length overflow"))?;
        let used = bytes.min(uncovered);
        if let DataRun::Data { lcn, .. } = *run {
            lcn.checked_mul(cluster_size)
                .and_then(|offset| offset.checked_add(used))
                .ok_or(MftError::InvalidDataRun("run offset overflow"))?;
        }
        uncovered -= used;
    }
    if uncovered != 0 {
        return Err(MftError::InvalidDataRun(
            "data runs shorter than requested attribute size",
        ));
    }
    let capacity = usize::try_from(data_size)
        .map_err(|_| MftError::InvalidDataRun("attribute data size exceeds addressable memory"))?;
    let mut out = Vec::new();
    out.try_reserve_exact(capacity)
        .map_err(|error| io_err(std::io::Error::other(error)))?;
    let mut remaining = data_size;
    for run in runs {
        if remaining == 0 {
            break;
        }
        match *run {
            DataRun::Data { lcn, clusters } => {
                let bytes = clusters
                    .checked_mul(cluster_size)
                    .ok_or(MftError::InvalidDataRun("run byte length overflow"))?;
                let to_read = bytes.min(remaining);
                let offset = lcn
                    .checked_mul(cluster_size)
                    .ok_or(MftError::InvalidDataRun("run offset overflow"))?;
                let start = out.len();
                let to_read = usize::try_from(to_read).map_err(|_| {
                    MftError::InvalidDataRun("run read size exceeds addressable memory")
                })?;
                let end = start
                    .checked_add(to_read)
                    .ok_or(MftError::InvalidDataRun("run read buffer overflow"))?;
                out.resize(end, 0);
                reader.seek(SeekFrom::Start(offset)).map_err(io_err)?;
                reader.read_exact(&mut out[start..]).map_err(io_err)?;
                remaining -= to_read as u64;
            }
            DataRun::Sparse { clusters } => {
                let bytes = clusters
                    .checked_mul(cluster_size)
                    .ok_or(MftError::InvalidDataRun("run byte length overflow"))?;
                let to_zero = bytes.min(remaining);
                let to_zero = usize::try_from(to_zero).map_err(|_| {
                    MftError::InvalidDataRun("sparse run size exceeds addressable memory")
                })?;
                let end = out
                    .len()
                    .checked_add(to_zero)
                    .ok_or(MftError::InvalidDataRun("sparse run buffer overflow"))?;
                out.resize(end, 0);
                remaining -= to_zero as u64;
            }
        }
    }
    if remaining != 0 {
        return Err(MftError::InvalidDataRun(
            "data runs shorter than requested attribute size",
        ));
    }
    Ok(out)
}

/// Convert a standard I/O error into the crate's public error type.
pub(super) fn io_err(error: std::io::Error) -> MftError {
    MftError::Io(error)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    use windows::Win32::Foundation::HANDLE;

    fn sparse_only_reader() -> VolumeReader {
        #[cfg(windows)]
        let volume = crate::volume::Volume::mock(
            HANDLE(std::ptr::null_mut()),
            crate::volume::VolumeSource::DriveLetter('C'),
        );
        #[cfg(target_os = "linux")]
        let volume =
            crate::volume::Volume::from_device_path("/dev/zero").expect("open test byte source");
        #[cfg(target_os = "macos")]
        let volume = crate::volume::Volume::from_device_path(std::env::current_exe().unwrap())
            .expect("open test byte source");
        VolumeReader::new(&volume, 512, crate::raw_mft::DEFAULT_BUFFER_BYTES)
            .expect("test reader construction should succeed")
    }

    #[test]
    fn read_nonresident_accepts_exact_sparse_coverage() {
        let mut reader = sparse_only_reader();
        let out = read_nonresident(&mut reader, &[DataRun::Sparse { clusters: 2 }], 4, 8)
            .expect("exact sparse coverage should succeed");
        assert_eq!(out, vec![0; 8]);
    }

    #[test]
    fn read_nonresident_rejects_short_sparse_coverage() {
        let mut reader = sparse_only_reader();
        let error =
            read_nonresident(&mut reader, &[DataRun::Sparse { clusters: 1 }], 4, 8).unwrap_err();
        assert!(matches!(
            error,
            MftError::InvalidDataRun("data runs shorter than requested attribute size")
        ));
    }

    #[test]
    fn read_nonresident_rejects_untrusted_allocation_sizes() {
        let mut reader = sparse_only_reader();
        for size in [u64::MAX, 64 * 1024 * 1024 + 1] {
            assert!(
                read_nonresident(&mut reader, &[DataRun::Sparse { clusters: size }], 1, size,)
                    .is_err()
            );
        }
    }

    #[test]
    fn read_nonresident_checks_coverage_before_io() {
        let mut reader = sparse_only_reader();
        let error = read_nonresident(
            &mut reader,
            &[DataRun::Data {
                lcn: 0,
                clusters: 1,
            }],
            4,
            8,
        )
        .unwrap_err();
        assert!(matches!(
            error,
            MftError::InvalidDataRun("data runs shorter than requested attribute size")
        ));
    }
}
