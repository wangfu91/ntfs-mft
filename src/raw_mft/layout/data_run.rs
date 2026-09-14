//! NTFS data-run decoder.
//!
//! Non-resident attributes' content is described by a sequence of "data
//! runs" that map a contiguous range of virtual cluster numbers (VCNs) to
//! either a logical cluster number (LCN) on the volume or to a sparse
//! hole. Each run is encoded as a header byte (high nibble = byte length
//! of the offset field, low nibble = byte length of the length field)
//! followed by those length and offset fields, the offset being a signed
//! delta from the previous run's LCN.

use crate::errors::MftError;

/// One decoded data run.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum DataRun {
    /// Allocated clusters starting at `lcn`.
    Data {
        /// Starting logical cluster number on disk.
        lcn: u64,
        /// Number of clusters in the run.
        clusters: u64,
    },
    /// Sparse hole occupying `clusters` logical clusters.
    Sparse {
        /// Number of logical clusters represented by the hole.
        clusters: u64,
    },
}

/// Maximum byte width of an NTFS run length or offset field.
const MAX_FIELD_BYTES: usize = 8;

fn read_var_u64_le(bytes: &[u8]) -> u64 {
    let mut buf = [0u8; MAX_FIELD_BYTES];
    buf[..bytes.len()].copy_from_slice(bytes);
    u64::from_le_bytes(buf)
}

fn read_var_i64_le(bytes: &[u8]) -> i64 {
    let mut buf = [0u8; MAX_FIELD_BYTES];
    buf[..bytes.len()].copy_from_slice(bytes);
    let raw = i64::from_le_bytes(buf);
    let empty_bits = (MAX_FIELD_BYTES - bytes.len()) * 8;
    // NTFS stores LCN deltas as variable-width signed little-endian values.
    // Shift left to place the encoded sign bit at i64's sign bit, then use
    // arithmetic right shift to sign-extend it back to the original width.
    (raw << empty_bits) >> empty_bits
}

/// Decode a sequence of data-run records starting at `runs`.
///
/// Returns the decoded sequence of runs.
pub(crate) fn decode_runs(runs: &[u8]) -> Result<Vec<DataRun>, MftError> {
    let mut out: Vec<DataRun> = Vec::new();
    visit_runs(runs, |run| {
        out.push(run);
        Ok(())
    })?;

    Ok(out)
}

/// Visit each decoded run in sequence.
fn visit_runs<F>(runs: &[u8], mut visitor: F) -> Result<(), MftError>
where
    F: FnMut(DataRun) -> Result<(), MftError>,
{
    let mut prev_lcn: i128 = 0;
    let mut cursor = 0usize;

    loop {
        if cursor >= runs.len() {
            return Err(MftError::InvalidDataRun("unterminated data run sequence"));
        }
        let descriptor = runs[cursor];
        if descriptor == 0 {
            break;
        }
        let length_bytes = (descriptor & 0x0F) as usize;
        let offset_bytes = ((descriptor >> 4) & 0x0F) as usize;
        if length_bytes == 0 || length_bytes > MAX_FIELD_BYTES {
            return Err(MftError::InvalidDataRun("invalid run length field"));
        }
        if offset_bytes > MAX_FIELD_BYTES {
            return Err(MftError::InvalidDataRun("invalid run offset field"));
        }
        cursor += 1;

        if cursor + length_bytes > runs.len() {
            return Err(MftError::InvalidDataRun("truncated run length field"));
        }
        let clusters = read_var_u64_le(&runs[cursor..cursor + length_bytes]);
        if clusters == 0 {
            return Err(MftError::InvalidDataRun("zero-length run"));
        }
        cursor += length_bytes;

        let lcn_opt = if offset_bytes == 0 {
            None
        } else {
            if cursor + offset_bytes > runs.len() {
                return Err(MftError::InvalidDataRun("truncated run offset field"));
            }
            let delta = read_var_i64_le(&runs[cursor..cursor + offset_bytes]);
            cursor += offset_bytes;
            let new_lcn = prev_lcn
                .checked_add(delta as i128)
                .ok_or(MftError::InvalidDataRun("relative offset overflow"))?;
            if new_lcn < 0 {
                return Err(MftError::InvalidDataRun("relative offset underflow"));
            }
            prev_lcn = new_lcn;
            Some(new_lcn as u64)
        };

        let run = match lcn_opt {
            Some(lcn) => DataRun::Data { lcn, clusters },
            None => DataRun::Sparse { clusters },
        };
        visitor(run)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_single_run() {
        // header 0x21: 1 byte length, 2 byte offset
        // length = 0x05, offset = 0x0234 -> LCN 564
        let runs = [0x21u8, 0x05, 0x34, 0x02, 0x00];
        let rs = decode_runs(&runs).unwrap();
        assert_eq!(
            rs,
            vec![DataRun::Data {
                lcn: 564,
                clusters: 5
            }]
        );
    }

    #[test]
    fn decodes_relative_offset_chain() {
        // first run: 0x21 len=5 off=10 -> lcn 10
        // second run: 0x21 len=3 off=-3 -> lcn 7
        let runs = [0x21u8, 0x05, 0x0A, 0x00, 0x21, 0x03, 0xFD, 0xFF, 0x00];
        let rs = decode_runs(&runs).unwrap();
        assert_eq!(
            rs,
            vec![
                DataRun::Data {
                    lcn: 10,
                    clusters: 5
                },
                DataRun::Data {
                    lcn: 7,
                    clusters: 3
                },
            ]
        );
    }

    #[test]
    fn decodes_sparse_run() {
        // header 0x01: 1 byte length, 0 byte offset -> sparse hole
        let runs = [0x01u8, 0x07, 0x00];
        let rs = decode_runs(&runs).unwrap();
        assert_eq!(rs, vec![DataRun::Sparse { clusters: 7 }]);
    }

    #[test]
    fn rejects_truncated_runs() {
        let runs = [0x21u8, 0x05]; // missing offset bytes + terminator
        assert!(decode_runs(&runs).is_err());
    }

    #[test]
    fn rejects_zero_length_run() {
        let runs = [0x11u8, 0x00, 0x00, 0x00];
        assert!(decode_runs(&runs).is_err());
    }

    #[test]
    fn rejects_negative_absolute_lcn() {
        // header 0x11 len=1 off=-5 -> previous lcn = 0, new lcn = -5
        let runs = [0x11u8, 0x01, 0xFB, 0x00];
        assert!(decode_runs(&runs).is_err());
    }

    #[test]
    fn lone_terminator_decodes_to_no_runs() {
        let runs = decode_runs(&[0x00]).expect("terminator only is valid");
        assert!(runs.is_empty());
    }

    #[test]
    fn empty_input_is_unterminated() {
        let error = decode_runs(&[]).unwrap_err();
        assert!(matches!(
            error,
            MftError::InvalidDataRun("unterminated data run sequence")
        ));
    }

    #[test]
    fn decodes_multi_byte_length_field() {
        // header 0x12: 2-byte length, 1-byte offset.
        // length = 0x0100 (256 clusters), offset = 0x05 -> LCN 5.
        let runs = [0x12u8, 0x00, 0x01, 0x05, 0x00];
        let decoded = decode_runs(&runs).expect("valid multi-byte length run");
        assert_eq!(
            decoded,
            vec![DataRun::Data {
                lcn: 5,
                clusters: 256
            }]
        );
    }

    #[test]
    fn decodes_eight_byte_offset_field() {
        // header 0x81: 1-byte length, 8-byte offset.
        // clusters = 2, LCN = 0x1234_5678.
        let runs = [
            0x81u8, 0x02, 0x78, 0x56, 0x34, 0x12, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        let decoded = decode_runs(&runs).expect("valid 8-byte offset run");
        assert_eq!(
            decoded,
            vec![DataRun::Data {
                lcn: 0x1234_5678,
                clusters: 2
            }]
        );
    }

    #[test]
    fn three_byte_negative_offset_sign_extends() {
        // First run: 0x31 len=2, offset=+0x100000 (3 bytes) -> LCN 1_048_576.
        // Second run: 0x31 len=1, offset=-1 (0xFFFFFF, 3 bytes) -> LCN 1_048_575.
        let runs = [
            0x31u8, 0x02, 0x00, 0x00, 0x10, 0x31, 0x01, 0xFF, 0xFF, 0xFF, 0x00,
        ];
        let decoded = decode_runs(&runs).expect("valid signed offset chain");
        assert_eq!(
            decoded,
            vec![
                DataRun::Data {
                    lcn: 1_048_576,
                    clusters: 2
                },
                DataRun::Data {
                    lcn: 1_048_575,
                    clusters: 1
                },
            ]
        );
    }

    #[test]
    fn sparse_run_does_not_shift_following_data_lcn() {
        // Sparse hole (offset nibble 0) must not move the LCN baseline, so the
        // following data run's delta is relative to LCN 0.
        // 0x01 len=4 -> sparse(4); 0x21 len=2 off=0x000A -> Data{lcn:10}.
        let runs = [0x01u8, 0x04, 0x21, 0x02, 0x0A, 0x00, 0x00];
        let decoded = decode_runs(&runs).expect("valid sparse-then-data chain");
        assert_eq!(
            decoded,
            vec![
                DataRun::Sparse { clusters: 4 },
                DataRun::Data {
                    lcn: 10,
                    clusters: 2
                },
            ]
        );
    }
}
