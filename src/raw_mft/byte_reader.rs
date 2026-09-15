//! A byte-oriented view of the logical `$MFT::$DATA` stream.

use super::{
    DEFAULT_BUFFER_BYTES, RawMft,
    inspection::RecordInspection,
    layout::{attribute::for_each_attribute_checked, extent::ExtentMap, record::FileRecord},
    volume_reader::VolumeReader,
};
use crate::{MftError, MftResult, Volume};
use std::io::{self, Read, Seek, SeekFrom};

/// Read-only `Read + Seek` adapter for a volume's logical `$MFT::$DATA` bytes.
///
/// Created by [`RawMft::byte_reader`]. It maps fragmented extents, preserves
/// original USA-protected bytes, and zero-fills sparse and uninitialized ranges.
/// It owns an independent volume handle and can outlive the original `RawMft`
/// and `Volume`. Logical geometry is captured at creation, not refreshed later.
/// A live volume is not an atomic snapshot; use a stable source for consistency.
/// This does not expose payloads of other files or external attribute lists.
pub struct MftByteReader {
    // Drop the borrowing Windows reader before the owning volume handle.
    inner: MappedReader<VolumeReader>,
    _volume: Volume,
}

impl RawMft<'_> {
    /// Open an independent, read-only logical `$MFT` byte stream.
    ///
    /// Unlike `record_count()` (extent capacity), this reader's EOF is the
    /// declared `$DATA` logical size. Compressed/encrypted MFT data is rejected.
    /// Sparse regions and bytes beyond initialized size read as zeros.
    ///
    /// ```no_run
    /// use ntfs_mft::{MftResult, MftStream, MftStreamOptions, Volume};
    /// # fn main() -> MftResult<()> {
    /// # #[cfg(windows)] {
    /// let volume = Volume::from_drive_letter('C')?;
    /// let raw = volume.raw_mft()?;
    /// let options = MftStreamOptions::new(raw.file_record_size())?
    ///     .resolve_extensions(true);
    /// let mut stream = MftStream::from_reader(raw.byte_reader()?, options)?;
    /// for record in stream.iter_records().take(100) {
    ///     println!("{}", record?.entry);
    /// }
    /// # }
    /// # Ok(())
    /// # }
    /// ```
    pub fn byte_reader(&self) -> MftResult<MftByteReader> {
        let volume = self.volume.reopen()?;
        let reader = VolumeReader::new(
            &volume,
            self.boot.bytes_per_sector as u64,
            DEFAULT_BUFFER_BYTES,
        )?;
        let mut inner = MappedReader::new(reader, self.extent_map.as_ref())?;
        // Read record zero through the same mapping, including a fragmented record.
        let mut bytes = vec![0; self.boot.file_record_size as usize];
        inner.read_exact(&mut bytes)?;
        let record = FileRecord::parse(0, None, &mut bytes)?;
        let (offset, used) = record.attrs_range();
        let mut size = None;
        let mut duplicate = false;
        let mut inspection = RecordInspection::default();
        let complete = for_each_attribute_checked(record.data, offset, used, |attr| {
            if attr.type_id() == 0x80
                && !attr.has_name()
                && inspection.observe(record.number, attr)
                && let Some(header) = attr.nonresident_header()
                && header.lowest_vcn == 0
            {
                duplicate |= size.is_some();
                size = Some((header.data_size, header.initialized_size, attr.flags()));
            }
        });
        if !complete || duplicate || inspection.has_malformed_attribute() {
            return Err(MftError::InvalidRecordData(
                "ambiguous or malformed MFT stream attributes",
            ));
        }
        let (size, initialized, flags) =
            size.ok_or(MftError::MftAttributeMissing("$MFT initial $DATA segment"))?;
        if flags & (0x0001 | 0x4000) != 0 {
            return Err(MftError::InvalidRecordData(
                "compressed or encrypted MFT byte streams are unsupported",
            ));
        }
        inner.set_lengths(size, initialized)?;
        inner.seek(SeekFrom::Start(0))?;
        Ok(MftByteReader {
            inner,
            _volume: volume,
        })
    }
}

impl MftByteReader {
    /// Logical byte length captured from `$MFT::$DATA`.
    pub fn len(&self) -> u64 {
        self.inner.length
    }
    /// Whether the logical stream contains no bytes.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Read for MftByteReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buf)
    }
}
impl Seek for MftByteReader {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.inner.seek(pos)
    }
    fn stream_position(&mut self) -> io::Result<u64> {
        Ok(self.inner.position)
    }
}

struct Segment {
    start: u64,
    end: u64,
    physical: Option<u64>,
}

// Generic internally so the exact production mapping is testable with a memory
// device, including arbitrary partial reads and injected OS failures.
pub(super) struct MappedReader<R> {
    reader: R,
    segments: Vec<Segment>,
    capacity: u64,
    length: u64,
    initialized: u64,
    position: u64,
}

impl<R> MappedReader<R> {
    pub(super) fn new(reader: R, map: &ExtentMap) -> MftResult<Self> {
        if map.cluster_size == 0 {
            return Err(MftError::InvalidDataRun("zero cluster size"));
        }
        let invalid = || MftError::InvalidDataRun("invalid or overflowing MFT byte extents");
        let mut segments = Vec::with_capacity(map.segments.len());
        let mut capacity = 0;
        for segment in &map.segments {
            let start = segment
                .vcn_start
                .checked_mul(map.cluster_size)
                .ok_or_else(invalid)?;
            let bytes = segment
                .clusters
                .checked_mul(map.cluster_size)
                .ok_or_else(invalid)?;
            let end = start.checked_add(bytes).ok_or_else(invalid)?;
            if bytes == 0 || start != capacity {
                return Err(invalid());
            }
            let physical = segment
                .lcn
                .map(|lcn| lcn.checked_mul(map.cluster_size).ok_or_else(invalid))
                .transpose()?;
            // VolumeReader's Windows seek uses a signed 64-bit offset.
            if let Some(physical) = physical
                && physical
                    .checked_add(bytes)
                    .is_none_or(|end| end > i64::MAX as u64)
            {
                return Err(invalid());
            }
            segments.push(Segment {
                start,
                end,
                physical,
            });
            capacity = end;
        }
        Ok(Self {
            reader,
            segments,
            capacity,
            length: capacity,
            initialized: capacity,
            position: 0,
        })
    }

    fn set_lengths(&mut self, length: u64, initialized: u64) -> MftResult<()> {
        if initialized > length || length > self.capacity {
            return Err(MftError::InvalidRecordData(
                "MFT data/initialized size exceeds mapped stream bounds",
            ));
        }
        self.length = length;
        self.initialized = initialized;
        Ok(())
    }
}

impl<R: Read + Seek> Read for MappedReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() || self.position >= self.length {
            return Ok(0);
        }
        let remaining = self.length - self.position;
        let requested = remaining.min(buf.len() as u64) as usize;
        if self.position >= self.initialized {
            buf[..requested].fill(0);
            self.position += requested as u64;
            return Ok(requested);
        }
        let index = self
            .segments
            .partition_point(|segment| segment.end <= self.position);
        let segment = self
            .segments
            .get(index)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "unmapped MFT offset"))?;
        let count = (requested as u64)
            .min(segment.end - self.position)
            .min(self.initialized - self.position) as usize;
        let read = if let Some(physical) = segment.physical {
            self.reader
                .seek(SeekFrom::Start(physical + (self.position - segment.start)))?;
            let count = self.reader.read(&mut buf[..count])?;
            if count == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "truncated MFT extent on volume",
                ));
            }
            count
        } else {
            buf[..count].fill(0);
            count
        };
        self.position += read as u64;
        Ok(read)
    }
}

impl<R> Seek for MappedReader<R> {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let position = match from {
            SeekFrom::Start(position) => Some(position),
            SeekFrom::Current(delta) => self.position.checked_add_signed(delta),
            SeekFrom::End(delta) => self.length.checked_add_signed(delta),
        }
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "MFT seek before zero or beyond u64 range",
            )
        })?;
        self.position = position;
        Ok(position)
    }
    fn stream_position(&mut self) -> io::Result<u64> {
        Ok(self.position)
    }
}

#[cfg(test)]
mod tests;
