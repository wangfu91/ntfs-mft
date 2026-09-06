use super::*;
use crate::raw_mft::layout::data_run::DataRun;
use std::io::Cursor;

fn fragmented() -> MftResult<MappedReader<Cursor<Vec<u8>>>> {
    let mut bytes = vec![0; 48];
    bytes[32..40].copy_from_slice(b"abcdefgh");
    bytes[8..16].copy_from_slice(b"ijklmnop");
    let map = ExtentMap::from_runs(
        &[
            DataRun::Data {
                lcn: 8,
                clusters: 2,
            },
            DataRun::Sparse { clusters: 1 },
            DataRun::Data {
                lcn: 2,
                clusters: 2,
            },
        ],
        4,
        8,
    );
    MappedReader::new(Cursor::new(bytes), &map)
}

#[test]
fn fragmented_reads_sparse_holes_and_logical_eof() -> MftResult<()> {
    let mut stream = fragmented()?;
    stream.set_lengths(19, 17)?;
    let mut all = Vec::new();
    stream.read_to_end(&mut all)?;
    assert_eq!(all, b"abcdefgh\0\0\0\0ijklm\0\0");
    stream.seek(SeekFrom::Start(6))?;
    let mut cross = [0; 9];
    stream.read_exact(&mut cross)?;
    assert_eq!(&cross, b"gh\0\0\0\0ijk");
    assert_eq!(stream.seek(SeekFrom::End(-2))?, 17);
    assert_eq!(stream.seek(SeekFrom::Current(-1))?, 16);
    assert_eq!(stream.seek(SeekFrom::End(100))?, 119);
    assert_eq!(stream.read(&mut cross)?, 0);
    assert!(stream.seek(SeekFrom::Start(u64::MAX)).is_ok());
    assert!(stream.seek(SeekFrom::Current(1)).is_err());
    assert_eq!(stream.stream_position()?, u64::MAX);
    stream.seek(SeekFrom::Start(0))?;
    assert!(stream.seek(SeekFrom::Current(-1)).is_err());
    assert_eq!(stream.stream_position()?, 0);
    assert_eq!(stream.read(&mut [])?, 0);
    Ok(())
}

#[test]
fn every_offset_and_read_length_matches_materialized_stream() -> MftResult<()> {
    let expected = b"abcdefgh\0\0\0\0ijklmnop";
    let mut stream = fragmented()?;
    for offset in (0..expected.len()).rev() {
        for length in 0..=expected.len() - offset {
            stream.seek(SeekFrom::Start(offset as u64))?;
            let mut bytes = vec![0; length];
            stream.read_exact(&mut bytes)?;
            assert_eq!(bytes, &expected[offset..offset + length]);
        }
    }
    Ok(())
}

#[test]
fn invalid_geometry_and_lengths_are_rejected() -> MftResult<()> {
    let mut stream = fragmented()?;
    assert!(stream.set_lengths(21, 20).is_err());
    assert!(stream.set_lengths(20, 21).is_err());
    for runs in [
        vec![DataRun::Data {
            lcn: u64::MAX,
            clusters: 1,
        }],
        vec![DataRun::Sparse { clusters: u64::MAX }],
        vec![DataRun::Sparse { clusters: 0 }],
    ] {
        let map = ExtentMap::from_runs(&runs, 4, 8);
        assert!(MappedReader::new(Cursor::new(Vec::<u8>::new()), &map).is_err());
    }
    let mut map = ExtentMap::from_runs(&[DataRun::Sparse { clusters: 1 }], 4, 8);
    map.segments[0].vcn_start = 1;
    assert!(MappedReader::new(Cursor::new(Vec::<u8>::new()), &map).is_err());
    let map = ExtentMap::from_runs(&[], 4, 8);
    let mut empty = MappedReader::new(Cursor::new(Vec::<u8>::new()), &map)?;
    assert_eq!(empty.read(&mut [0])?, 0);
    Ok(())
}

struct Faulty {
    bytes: Cursor<Vec<u8>>,
    fail_seek: bool,
    fail_read: bool,
}
impl Seek for Faulty {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        if self.fail_seek {
            return Err(io::Error::other("seek failure"));
        }
        self.bytes.seek(pos)
    }
}
impl Read for Faulty {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.fail_read {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "read failure",
            ));
        }
        let count = buf.len().min(1);
        self.bytes.read(&mut buf[..count])
    }
}

#[test]
fn short_reads_io_errors_and_unexpected_eof_preserve_cursor() -> MftResult<()> {
    let map = ExtentMap::from_runs(
        &[DataRun::Data {
            lcn: 0,
            clusters: 4,
        }],
        1,
        4,
    );
    let mut stream = MappedReader::new(
        Faulty {
            bytes: Cursor::new(b"abc".to_vec()),
            fail_seek: true,
            fail_read: false,
        },
        &map,
    )?;
    assert!(stream.read(&mut [0; 4]).is_err());
    assert_eq!(stream.position, 0);
    stream.reader.fail_seek = false;
    stream.reader.fail_read = true;
    assert_eq!(
        stream.read(&mut [0; 4]).unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
    assert_eq!(stream.position, 0);
    stream.reader.fail_read = false;
    let mut bytes = [0; 3];
    stream.read_exact(&mut bytes)?;
    assert_eq!(&bytes, b"abc");
    assert_eq!(
        stream.read(&mut [0]).unwrap_err().kind(),
        io::ErrorKind::UnexpectedEof
    );
    assert_eq!(stream.position, 3);
    Ok(())
}

#[test]
fn mft_stream_parses_records_split_across_physical_extents() -> MftResult<()> {
    use crate::{MftStream, MftStreamOptions};
    let fixture = include_bytes!("../../../tests/fixtures/omerbenamram-mft/MFT");
    // Split record zero in half, scatter it out of physical order, then append
    // record one. This exercises boundaries *within* a USA-protected record.
    let mut device = vec![0; 4096];
    device[3072..3584].copy_from_slice(&fixture[..512]);
    device[512..2048].copy_from_slice(&fixture[512..2048]);
    let map = ExtentMap::from_runs(
        &[
            DataRun::Data {
                lcn: 6,
                clusters: 1,
            },
            DataRun::Data {
                lcn: 1,
                clusters: 3,
            },
        ],
        512,
        1024,
    );
    let reader = MappedReader::new(Cursor::new(device), &map)?;
    let options = MftStreamOptions::new(1024)?;
    let mut mapped = MftStream::from_reader(reader, options)?;
    let mut plain = MftStream::from_reader(Cursor::new(&fixture[..2048]), options)?;
    for number in [1, 0, 1] {
        assert_eq!(
            mapped.read_record(number)?.entry,
            plain.read_record(number)?.entry
        );
    }
    assert_eq!(
        mapped.iter_records().collect::<MftResult<Vec<_>>>()?.len(),
        2
    );
    Ok(())
}
