//! Extent map mapping virtual cluster numbers (VCNs) of the `$MFT`
//! `$DATA` attribute to volume byte offsets.
//!
//! NTFS keeps the `$MFT` as a regular file, so its data can be split across
//! multiple extents and may even contain sparse holes. This module turns the
//! decoded `$MFT` runlist into a lookup structure that can answer:
//!
//! `record number -> absolute disk byte offset`

use crate::{errors::MftError, raw_mft::layout::data_run::DataRun};

/// Cached segment index used to accelerate mostly-sequential extent lookups.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ExtentLookupCursor {
    segment_index: usize,
}

/// Complete physical coverage of one record. Only split records allocate.
pub(crate) enum RecordLocation {
    Contiguous { offset: u64, len: usize },
    Split(Vec<(u64, usize)>),
}

/// One contiguous VCN range in the `$MFT::$DATA` runlist.
#[derive(Debug, Clone)]
pub(crate) struct ExtentSegment {
    /// First VCN covered by this segment.
    pub vcn_start: u64,
    /// Number of clusters in this segment.
    pub clusters: u64,
    /// LCN at `vcn_start`, or `None` for sparse holes.
    pub lcn: Option<u64>,
}

/// Runlist-derived mapping from logical FILE record numbers to absolute
/// volume offsets.
#[derive(Debug, Clone)]
pub(crate) struct ExtentMap {
    /// Ordered list of VCN-to-LCN segments.
    pub segments: Vec<ExtentSegment>,
    /// Cluster size in bytes.
    pub cluster_size: u64,
    /// FILE-record size in bytes.
    pub file_record_size: u64,
    /// Total number of logical clusters across all segments.
    pub total_clusters: u64,
}

impl ExtentMap {
    /// Map every byte of a record, rejecting overflow or missing tail coverage.
    /// A record touching a sparse hole has no usable raw FILE-record location.
    pub(crate) fn record_location(
        &self,
        number: u64,
        cursor: &mut ExtentLookupCursor,
    ) -> Result<Option<RecordLocation>, MftError> {
        if self.cluster_size == 0 || self.file_record_size == 0 {
            return Err(MftError::InvalidDataRun("zero-sized record geometry"));
        }
        let Some(first_offset) = self.record_offset_with_cursor(number, cursor)? else {
            return Ok(None);
        };
        let invalid = || MftError::InvalidDataRun("record extent range overflow or gap");
        let mut logical = number
            .checked_mul(self.file_record_size)
            .ok_or_else(invalid)?;
        let mut remaining = self.file_record_size;
        let mut parts = Vec::new();
        for segment in &self.segments[cursor.segment_index..] {
            let start = segment
                .vcn_start
                .checked_mul(self.cluster_size)
                .ok_or_else(invalid)?;
            let length = segment
                .clusters
                .checked_mul(self.cluster_size)
                .ok_or_else(invalid)?;
            let end = start.checked_add(length).ok_or_else(invalid)?;
            if logical < start || logical >= end {
                return Err(invalid());
            }
            let Some(lcn) = segment.lcn else {
                return Ok(None);
            };
            let offset = lcn
                .checked_mul(self.cluster_size)
                .and_then(|base| base.checked_add(logical - start))
                .ok_or_else(invalid)?;
            let count = remaining.min(end - logical);
            offset.checked_add(count).ok_or_else(invalid)?;
            let len = usize::try_from(count).map_err(|_| invalid())?;
            if parts.is_empty() && count == self.file_record_size {
                return Ok(Some(RecordLocation::Contiguous {
                    offset: first_offset,
                    len,
                }));
            }
            parts.push((offset, len));
            logical = logical.checked_add(count).ok_or_else(invalid)?;
            remaining -= count;
            if remaining == 0 {
                return Ok(Some(RecordLocation::Split(parts)));
            }
        }
        Err(MftError::InvalidMftRecord {
            number,
            reason: "record extends beyond MFT extent map",
        })
    }

    /// Build an extent map from the decoded runs of the `$MFT` data stream.
    pub fn from_runs(runs: &[DataRun], cluster_size: u64, file_record_size: u64) -> Self {
        let mut segments = Vec::with_capacity(runs.len());
        let mut vcn_start: u64 = 0;
        let mut total_clusters: u64 = 0;
        for run in runs {
            match *run {
                DataRun::Data { lcn, clusters } => {
                    segments.push(ExtentSegment {
                        vcn_start,
                        clusters,
                        lcn: Some(lcn),
                    });
                    vcn_start = vcn_start.saturating_add(clusters);
                    total_clusters = total_clusters.saturating_add(clusters);
                }
                DataRun::Sparse { clusters } => {
                    segments.push(ExtentSegment {
                        vcn_start,
                        clusters,
                        lcn: None,
                    });
                    vcn_start = vcn_start.saturating_add(clusters);
                    total_clusters = total_clusters.saturating_add(clusters);
                }
            }
        }
        ExtentMap {
            segments,
            cluster_size,
            file_record_size,
            total_clusters,
        }
    }

    /// Total number of records that the MFT data can hold (including sparse
    /// holes in the logical address space).
    pub fn record_count(&self) -> u64 {
        if self.file_record_size == 0 {
            return 0;
        }
        self.total_clusters.saturating_mul(self.cluster_size) / self.file_record_size
    }

    /// Translate a record number to its absolute volume byte offset.
    ///
    /// This is the convenient one-shot form. It creates a fresh lookup
    /// cursor internally, so it is simplest to use when you do not care
    /// about repeated nearby lookups.
    ///
    /// Returns `Ok(None)` for sparse regions and `Err` for out-of-range
    /// record numbers.
    #[cfg(test)]
    pub fn record_offset(&self, record_number: u64) -> Result<Option<u64>, MftError> {
        let mut cursor = ExtentLookupCursor::default();
        self.record_offset_with_cursor(record_number, &mut cursor)
    }

    /// Translate a record number to its absolute volume byte offset while
    /// reusing a caller-owned lookup cursor across nearby queries.
    ///
    /// Use this when scanning records in order, because the cached segment
    /// index lets the next lookup start near the previous one instead of
    /// rescanning the entire extent list from the beginning.
    pub fn record_offset_with_cursor(
        &self,
        record_number: u64,
        cursor: &mut ExtentLookupCursor,
    ) -> Result<Option<u64>, MftError> {
        let byte_off = record_number
            .checked_mul(self.file_record_size)
            .ok_or(MftError::InvalidDataRun("record offset overflow"))?;
        let vcn = byte_off / self.cluster_size;
        let inner = byte_off % self.cluster_size;
        let start_index = match self.segments.get(cursor.segment_index) {
            Some(seg) => {
                let end = seg.vcn_start.saturating_add(seg.clusters);
                if vcn < seg.vcn_start {
                    0
                } else if vcn < end {
                    cursor.segment_index
                } else {
                    cursor.segment_index.saturating_add(1)
                }
            }
            None => 0,
        };
        for (index, seg) in self.segments.iter().enumerate().skip(start_index) {
            let end = seg.vcn_start.saturating_add(seg.clusters);
            if vcn < seg.vcn_start {
                break;
            }
            if vcn >= seg.vcn_start && vcn < end {
                cursor.segment_index = index;
                let local_vcn = vcn - seg.vcn_start;
                return seg
                    .lcn
                    .map(|lcn| {
                        lcn.checked_add(local_vcn)
                            .and_then(|cluster| cluster.checked_mul(self.cluster_size))
                            .and_then(|offset| offset.checked_add(inner))
                            .ok_or(MftError::InvalidDataRun("volume offset overflow"))
                    })
                    .transpose();
            }
        }
        Err(MftError::InvalidMftRecord {
            number: record_number,
            reason: "record number outside MFT extent map",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_record_mapping_rejects_missing_sparse_and_overflowing_tails() {
        let mut cursor = ExtentLookupCursor::default();
        let map = ExtentMap::from_runs(
            &[DataRun::Data {
                lcn: 2,
                clusters: 1,
            }],
            512,
            1024,
        );
        assert!(map.record_location(0, &mut cursor).is_err());
        let map = ExtentMap::from_runs(
            &[
                DataRun::Data {
                    lcn: 2,
                    clusters: 1,
                },
                DataRun::Sparse { clusters: 1 },
            ],
            512,
            1024,
        );
        assert!(map.record_location(0, &mut cursor).unwrap().is_none());
        let map = ExtentMap::from_runs(
            &[DataRun::Data {
                lcn: u64::MAX,
                clusters: 1,
            }],
            1,
            1,
        );
        assert!(map.record_location(0, &mut cursor).is_err());
    }

    #[test]
    fn complete_record_mapping_preserves_contiguous_fast_path_and_split_order() {
        let map = ExtentMap::from_runs(
            &[
                DataRun::Data {
                    lcn: 2,
                    clusters: 3,
                },
                DataRun::Data {
                    lcn: 10,
                    clusters: 1,
                },
            ],
            512,
            1024,
        );
        let mut cursor = ExtentLookupCursor::default();
        assert!(matches!(
            map.record_location(0, &mut cursor).unwrap(),
            Some(RecordLocation::Contiguous {
                offset: 1024,
                len: 1024
            })
        ));
        let Some(RecordLocation::Split(parts)) = map.record_location(1, &mut cursor).unwrap()
        else {
            panic!("expected split record");
        };
        assert_eq!(parts, vec![(2048, 512), (5120, 512)]);
        assert!(matches!(
            map.record_location(0, &mut cursor).unwrap(),
            Some(RecordLocation::Contiguous {
                offset: 1024,
                len: 1024
            })
        ));
    }

    #[test]
    fn maps_record_numbers_in_single_run() {
        let runs = vec![DataRun::Data {
            lcn: 100,
            clusters: 4,
        }];
        let map = ExtentMap::from_runs(&runs, 4096, 1024);
        // 4 records per cluster
        assert_eq!(map.record_count(), 16);
        assert_eq!(map.record_offset(0).unwrap(), Some(100 * 4096));
        assert_eq!(map.record_offset(3).unwrap(), Some(100 * 4096 + 3 * 1024));
        assert_eq!(map.record_offset(4).unwrap(), Some(101 * 4096));
    }

    #[test]
    fn maps_record_numbers_across_multiple_runs() {
        let runs = vec![
            DataRun::Data {
                lcn: 100,
                clusters: 2,
            }, // VCN 0..2
            DataRun::Data {
                lcn: 200,
                clusters: 3,
            }, // VCN 2..5
        ];
        let map = ExtentMap::from_runs(&runs, 4096, 1024);
        // record 8 -> VCN 2 (start of run 2) -> LCN 200
        assert_eq!(map.record_offset(8).unwrap(), Some(200 * 4096));
        // record 12 -> VCN 3 -> LCN 201
        assert_eq!(map.record_offset(12).unwrap(), Some(201 * 4096));
    }

    #[test]
    fn cursor_tracks_sequential_records_across_runs() {
        let runs = vec![
            DataRun::Data {
                lcn: 100,
                clusters: 2,
            },
            DataRun::Data {
                lcn: 200,
                clusters: 3,
            },
        ];
        let map = ExtentMap::from_runs(&runs, 4096, 1024);
        let mut cursor = ExtentLookupCursor::default();

        assert_eq!(
            map.record_offset_with_cursor(7, &mut cursor).unwrap(),
            Some(101 * 4096 + 3 * 1024)
        );
        assert_eq!(
            map.record_offset_with_cursor(8, &mut cursor).unwrap(),
            Some(200 * 4096)
        );
        assert_eq!(
            map.record_offset_with_cursor(12, &mut cursor).unwrap(),
            Some(201 * 4096)
        );
    }

    #[test]
    fn cursor_falls_back_for_earlier_records() {
        let runs = vec![
            DataRun::Data {
                lcn: 100,
                clusters: 2,
            },
            DataRun::Data {
                lcn: 200,
                clusters: 3,
            },
        ];
        let map = ExtentMap::from_runs(&runs, 4096, 1024);
        let mut cursor = ExtentLookupCursor::default();

        assert_eq!(
            map.record_offset_with_cursor(12, &mut cursor).unwrap(),
            Some(201 * 4096)
        );
        assert_eq!(
            map.record_offset_with_cursor(0, &mut cursor).unwrap(),
            Some(100 * 4096)
        );
    }

    #[test]
    fn returns_none_for_sparse_holes() {
        let runs = vec![
            DataRun::Sparse { clusters: 2 },
            DataRun::Data {
                lcn: 500,
                clusters: 1,
            },
        ];
        let map = ExtentMap::from_runs(&runs, 4096, 1024);
        assert_eq!(map.record_offset(0).unwrap(), None);
        assert_eq!(map.record_offset(8).unwrap(), Some(500 * 4096));
    }

    #[test]
    fn out_of_range_returns_error() {
        let runs = vec![DataRun::Data {
            lcn: 100,
            clusters: 1,
        }];
        let map = ExtentMap::from_runs(&runs, 4096, 1024);
        assert!(map.record_offset(100).is_err());
    }

    #[test]
    fn rejects_volume_offset_overflow() {
        let runs = vec![DataRun::Data {
            lcn: u64::MAX,
            clusters: 1,
        }];
        let map = ExtentMap::from_runs(&runs, 4096, 1024);

        assert!(matches!(
            map.record_offset(0),
            Err(MftError::InvalidDataRun("volume offset overflow"))
        ));
    }

    #[test]
    fn record_count_includes_sparse_holes() {
        // Sparse clusters still occupy logical address space, so they count
        // toward the number of addressable records.
        let runs = vec![
            DataRun::Sparse { clusters: 2 },
            DataRun::Data {
                lcn: 500,
                clusters: 2,
            },
        ];
        let map = ExtentMap::from_runs(&runs, 4096, 1024);
        // 4 clusters * 4096 bytes / 1024-byte records = 16 records.
        assert_eq!(map.record_count(), 16);
    }

    #[test]
    fn record_count_is_zero_when_file_record_size_is_zero() {
        let runs = vec![DataRun::Data {
            lcn: 100,
            clusters: 4,
        }];
        let map = ExtentMap::from_runs(&runs, 4096, 0);
        assert_eq!(map.record_count(), 0);
    }

    #[test]
    fn resolves_sub_cluster_offset_within_data_run() {
        // Record 9 lands inside the data run (VCN 2) at a 1024-byte sub-cluster
        // offset, exercising the intra-cluster remainder arithmetic.
        let runs = vec![
            DataRun::Sparse { clusters: 2 },
            DataRun::Data {
                lcn: 500,
                clusters: 2,
            },
        ];
        let map = ExtentMap::from_runs(&runs, 4096, 1024);
        assert_eq!(map.record_offset(9).unwrap(), Some(500 * 4096 + 1024));
    }
}
