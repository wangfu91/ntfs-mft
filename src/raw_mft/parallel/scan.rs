//! Builder-style public facade for parallel raw-MFT chunk scans.

use std::{num::NonZeroUsize, thread};

use crate::{
    errors::MftError,
    raw_mft::{
        RawMft, RawMftBatchEntry, RawMftChunkBatch, RawMftChunkPlanOptions, RawMftScanOptions,
        RawMftWorkChunk, parallel::ChunkScheduling,
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

/// Configured parallel raw `$MFT` scan.
#[derive(Clone)]
#[must_use]
pub struct RawMftParallelScan<'m, 'v> {
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
            mft,
            chunk_plan: RawMftChunkPlanOptions::default(),
            chunks: None,
            scan_options: RawMftScanOptions::default(),
            worker_count: None,
            scheduling: RawMftParallelScheduling::Dynamic,
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
    /// Omitted chunks are never recreated.
    pub fn chunks(mut self, chunks: Vec<RawMftWorkChunk>) -> Self {
        self.chunks = Some(chunks);
        self
    }

    /// Set record selection and parsing options for the entire scan.
    ///
    /// The range and unused-record policy apply to both automatic planning and
    /// execution, regardless of the order of builder calls. Explicit chunks
    /// further restrict the selected range.
    pub fn scan_options(mut self, options: RawMftScanOptions) -> Self {
        self.scan_options = options;
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

    /// Parse chunks in parallel and visit ordered batches.
    pub fn for_each_batch<F>(self, visit: F) -> Result<(), MftError>
    where
        F: FnMut(RawMftChunkBatch) -> Result<(), MftError>,
    {
        let worker_count = self.resolved_worker_count()?;
        self.mft.for_each_chunk(
            self.resolved_chunks()?,
            self.scan_options,
            worker_count,
            self.scheduling.into(),
            visit,
        )
    }

    /// Parse chunks in parallel, map each batch on the worker thread, and visit mapped values in chunk order.
    pub fn map_chunks<F, T, V>(self, map_chunk: F, visit: V) -> Result<(), MftError>
    where
        F: Fn(RawMftChunkBatch) -> Result<T, MftError> + Sync,
        T: Send,
        V: FnMut(T) -> Result<(), MftError>,
    {
        let worker_count = self.resolved_worker_count()?;
        self.mft.for_each_mapped_chunk(
            self.resolved_chunks()?,
            self.scan_options,
            worker_count,
            self.scheduling.into(),
            map_chunk,
            visit,
        )
    }

    /// Parse chunks in parallel and fold entries into one worker-local accumulator per chunk.
    pub fn fold_chunks<Init, Fold, T, V>(
        self,
        init: Init,
        fold_entry: Fold,
        visit: V,
    ) -> Result<(), MftError>
    where
        Init: Fn(RawMftWorkChunk) -> T + Sync,
        Fold: Fn(&mut T, RawMftBatchEntry) -> Result<(), MftError> + Sync,
        T: Send,
        V: FnMut(T) -> Result<(), MftError>,
    {
        let worker_count = self.resolved_worker_count()?;
        self.mft.for_each_folded_chunk(
            self.resolved_chunks()?,
            self.scan_options,
            worker_count,
            self.scheduling.into(),
            init,
            fold_entry,
            visit,
        )
    }

    /// Parse chunks in parallel with deferred, cache-backed `$ATTRIBUTE_LIST`
    /// enrichment, folding entries into worker-local accumulators.
    ///
    /// Extension records are captured during the scan and used to enrich the
    /// base records that need them in a second phase, avoiding the scattered
    /// random reads that inline enrichment performs. `init` takes no chunk
    /// because it also seeds the phase-2 accumulator. Worker scheduling is always
    /// dynamic for this mode.
    pub fn fold_records_cached_enrich<Init, Fold, T, V>(
        self,
        init: Init,
        fold_entry: Fold,
        visit: V,
    ) -> Result<(), MftError>
    where
        Init: Fn() -> T + Sync,
        Fold: Fn(&mut T, RawMftBatchEntry) -> Result<(), MftError> + Sync,
        T: Send,
        V: FnMut(T) -> Result<(), MftError>,
    {
        let worker_count = self.resolved_worker_count()?;
        self.mft.fold_records_cached_enrich(
            self.resolved_chunks()?,
            self.scan_options,
            worker_count,
            init,
            fold_entry,
            visit,
        )
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
                let chunks: Vec<_> = chunks
                    .iter()
                    .filter_map(|chunk| {
                        let start_record = chunk.start_record.max(range.start_record());
                        let end_record = chunk.end_record.min(end);
                        (start_record < end_record).then_some(RawMftWorkChunk {
                            start_record,
                            end_record,
                        })
                    })
                    .collect();
                // Validate a sorted copy, retaining caller order for execution.
                // Overlapping reads could also reapply USA fixups in reused buffers.
                let mut ordered = chunks.clone();
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
            None => Ok(self
                .mft
                .plan_chunks_with_options(self.chunk_plan.clone(), &self.scan_options)),
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

impl<'v> RawMft<'v> {
    /// Configure a parallel raw `$MFT` chunk scan.
    #[must_use = "parallel scan builders do nothing unless executed"]
    pub fn parallel(&self) -> RawMftParallelScan<'_, 'v> {
        RawMftParallelScan::new(self)
    }
}

#[cfg(test)]
mod tests;
