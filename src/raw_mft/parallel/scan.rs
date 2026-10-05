//! Builder-style public facade for parallel raw-MFT chunk scans.

use std::{marker::PhantomData, num::NonZeroUsize, thread};

use crate::{
    errors::MftError,
    raw_mft::{
        RawMft, RawMftChunkBatch, RawMftChunkPlanOptions, RawMftEntry, RawMftLeanEntry,
        RawMftLeanScanOptions, RawMftScanOptions, RawMftWorkChunk, parallel::ChunkScheduling,
    },
};

use super::executor;

/// Worker scheduling policy for parallel raw-`$MFT` chunk scans.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawMftParallelScheduling {
    /// Workers fetch the next remaining chunk from a shared atomic cursor.
    Dynamic,
    /// Each worker receives one contiguous band of chunk indices.
    Contiguous,
}

impl From<RawMftParallelScheduling> for ChunkScheduling {
    fn from(value: RawMftParallelScheduling) -> Self {
        match value {
            RawMftParallelScheduling::Dynamic => ChunkScheduling::Dynamic,
            RawMftParallelScheduling::Contiguous => ChunkScheduling::Contiguous,
        }
    }
}

/// Metadata projection has not yet been selected.
#[derive(Debug, Clone)]
pub struct Unselected;
/// Full entry metadata, matching rich serial iteration.
#[derive(Debug, Clone)]
pub struct Rich;
/// Reduced entry metadata for high-throughput scans.
#[derive(Debug, Clone)]
pub struct Lean;

/// Configured parallel raw `$MFT` scan. Choose `rich()` or `lean()`
/// before supplying projection-specific options and executing.
///
/// ```no_run
/// use ntfs_mft::{RawMft, RawMftLeanScanOptions, MftResult};
/// fn scan(mft: &RawMft<'_>) -> MftResult<()> {
///     let rich = mft.parallel().rich().collect_batches()?;
///     let lean = mft.parallel().lean()
///         .scan_options(RawMftLeanScanOptions::builder().include_unused_records(true).build())
///         .collect_batches()?;
///     Ok(())
/// }
/// ```
///
/// Selecting a projection is required:
/// ```compile_fail
/// # fn scan(mft: &ntfs_mft::RawMft<'_>) {
/// mft.parallel().collect_batches();
/// # }
/// ```
/// Rich-only collection settings cannot be passed to a lean scan:
/// ```compile_fail
/// # fn scan(mft: &ntfs_mft::RawMft<'_>) {
/// mft.parallel().lean().scan_options(ntfs_mft::RawMftScanOptions::default());
/// # }
/// ```
#[derive(Debug, Clone)]
#[must_use]
pub struct RawMftParallelScan<'m, 'v, P = Unselected> {
    projection: PhantomData<P>,
    mft: &'m RawMft<'v>,
    chunk_plan: RawMftChunkPlanOptions,
    chunks: Option<Vec<RawMftWorkChunk>>,
    scan_options: RawMftScanOptions,
    worker_count: Option<NonZeroUsize>,
    scheduling: RawMftParallelScheduling,
}

impl<'m, 'v> RawMftParallelScan<'m, 'v> {
    pub(crate) fn new(mft: &'m RawMft<'v>) -> Self {
        Self {
            projection: PhantomData,
            mft,
            chunk_plan: RawMftChunkPlanOptions::default(),
            chunks: None,
            scan_options: RawMftScanOptions::default(),
            worker_count: None,
            scheduling: RawMftParallelScheduling::Dynamic,
        }
    }

    /// Select full metadata, matching serial `RawMftEntry` results.
    pub fn rich(self) -> RawMftParallelScan<'m, 'v, Rich> {
        self.project()
    }
    /// Select lean metadata with lean-specific scan options.
    pub fn lean(self) -> RawMftParallelScan<'m, 'v, Lean> {
        self.project()
    }
}

impl<'m, 'v, P> RawMftParallelScan<'m, 'v, P> {
    fn project<Q>(self) -> RawMftParallelScan<'m, 'v, Q> {
        RawMftParallelScan {
            projection: PhantomData,
            mft: self.mft,
            chunk_plan: self.chunk_plan,
            chunks: self.chunks,
            scan_options: self.scan_options,
            worker_count: self.worker_count,
            scheduling: self.scheduling,
        }
    }
    /// Set partition sizing and switch back to automatic chunk planning.
    /// Selection always comes from [`Self::scan_options`].
    pub fn chunk_plan(mut self, options: RawMftChunkPlanOptions) -> Self {
        self.chunk_plan = options;
        self.chunks = None;
        self
    }

    /// Use explicit candidate chunks instead of automatic planning.
    ///
    /// Chunks are intersected with the scan range and volume bounds; empty
    /// intersections are discarded. Supplied order is preserved. Overlapping
    /// intersections are rejected with [`MftError::InvalidOptions`] before any
    /// records are scanned. Unused records are filtered using the scan options.
    /// Omitted chunks are never recreated. At execution, more than 4,194,304
    /// nonempty intersections are rejected before copying the plan.
    pub fn chunks(mut self, chunks: Vec<RawMftWorkChunk>) -> Self {
        self.chunks = Some(chunks);
        self
    }

    /// Use a fixed worker count. Defaults to `thread::available_parallelism()`.
    pub fn workers(mut self, worker_count: NonZeroUsize) -> Self {
        self.worker_count = Some(worker_count);
        self
    }

    /// Select the worker scheduling policy for this scan.
    pub fn scheduling(mut self, scheduling: RawMftParallelScheduling) -> Self {
        self.scheduling = scheduling;
        self
    }

    /// Resolve the work chunks for this scan, either from explicit chunks or by planning them from the chunk plan options.
    fn resolved_chunks(&self) -> Result<Vec<RawMftWorkChunk>, MftError> {
        match &self.chunks {
            Some(chunks) => {
                let range = self.scan_options.range();
                let end = range
                    .end_record()
                    .unwrap_or(self.mft.record_count())
                    .min(self.mft.record_count());
                let selected = chunks.iter().filter_map(|chunk| {
                    let start_record = chunk.start_record.max(range.start_record());
                    let end_record = chunk.end_record.min(end);
                    (start_record < end_record).then_some(RawMftWorkChunk {
                        start_record,
                        end_record,
                    })
                });
                let mut chunks = crate::raw_mft::chunk_plan::allocate_chunk_plan(
                    selected.clone().count() as u64,
                )?;
                chunks.extend(selected);
                // Validate a sorted copy, retaining caller order for execution.
                // Overlapping reads could also reapply USA fixups in reused buffers.
                let mut ordered =
                    crate::raw_mft::chunk_plan::allocate_chunk_plan(chunks.len() as u64)?;
                ordered.extend_from_slice(&chunks);
                ordered.sort_unstable_by_key(|chunk| chunk.start_record);
                if ordered
                    .windows(2)
                    .any(|pair| pair[0].end_record > pair[1].start_record)
                {
                    return Err(MftError::InvalidOptions(
                        "explicit chunks overlap within the scan range",
                    ));
                }
                Ok(chunks)
            }
            None => self
                .mft
                .plan_chunks_with_options(self.chunk_plan.clone(), &self.scan_options),
        }
    }

    /// Resolve the worker count for this scan, either from an explicit value or from `thread::available_parallelism()`.
    fn resolved_worker_count(&self) -> Result<NonZeroUsize, MftError> {
        match self.worker_count {
            Some(worker_count) => Ok(worker_count),
            None => thread::available_parallelism().map_err(executor::available_parallelism_error),
        }
    }
}

impl<'m, 'v> RawMftParallelScan<'m, 'v, Lean> {
    /// Set record selection and parsing options for the entire scan.
    ///
    /// The range and unused-record policy apply to both automatic planning and
    /// execution, regardless of the order of builder calls. Explicit chunks
    /// further restrict the selected range.
    pub fn scan_options(mut self, options: RawMftLeanScanOptions) -> Self {
        self.scan_options = options.inner;
        self
    }

    /// Parse chunks in parallel and collect ordered batches.
    pub fn collect_batches(self) -> Result<Vec<RawMftChunkBatch>, MftError> {
        let worker_count = self.resolved_worker_count()?;
        self.mft.read_chunks(
            self.resolved_chunks()?,
            self.scan_options,
            worker_count,
            self.scheduling.into(),
        )
    }
}

impl<'v> RawMft<'v> {
    /// Configure a parallel raw `$MFT` chunk scan.
    #[must_use = "parallel scan builders do nothing unless executed"]
    pub fn parallel(&self) -> RawMftParallelScan<'_, 'v> {
        RawMftParallelScan::new(self)
    }
}

#[cfg(test)]
mod tests;

impl<'m, 'v> RawMftParallelScan<'m, 'v, Rich> {
    /// Configure full metadata and shared record selection.
    pub fn scan_options(mut self, options: RawMftScanOptions) -> Self {
        self.scan_options = options;
        self
    }
    /// Collect full-metadata batches in supplied chunk order.
    pub fn collect_batches(self) -> Result<Vec<RawMftChunkBatch<RawMftEntry>>, MftError> {
        let mut result = Vec::new();
        match self.try_for_each_batch(|batch| {
            result.push(batch);
            Ok::<_, std::convert::Infallible>(())
        }) {
            Ok(()) => Ok(result),
            Err(crate::ScanError::Scanner(error)) => Err(error),
            Err(crate::ScanError::Consumer(never)) => match never {},
        }
    }
}

mod callbacks;
