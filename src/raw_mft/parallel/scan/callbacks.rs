//! Terminal scan operations with application-defined error types.
use super::*;
use crate::ScanError;

impl<'m, 'v> RawMftParallelScan<'m, 'v, Rich> {
    /// Execute the scan and visit batches on the calling thread, preserving
    /// application errors as `ScanError::Consumer`. Returning an error stops
    /// delivery; already-running worker callbacks may finish before return.
    pub fn try_for_each_batch<E, F>(self, visit: F) -> Result<(), ScanError<E>>
    where
        E: Send,
        F: FnMut(RawMftChunkBatch<RawMftEntry>) -> Result<(), E>,
    {
        self.try_map_chunks_for_each(Ok, visit)
    }
    /// Execute mapping on workers and visit mapped values in supplied chunk
    /// order. Both callback error types are preserved. This is a terminal
    /// operation, not a lazy iterator adapter.
    pub fn try_map_chunks_for_each<E, F, T, V>(
        self,
        map: F,
        mut visit: V,
    ) -> Result<(), ScanError<E>>
    where
        E: Send,
        F: Fn(RawMftChunkBatch<RawMftEntry>) -> Result<T, E> + Sync,
        T: Send,
        V: FnMut(T) -> Result<(), E>,
    {
        let workers = self.resolved_worker_count()?;
        executor::run_parallel_chunks_in_order(
            self.mft,
            self.resolved_chunks()?,
            self.scan_options,
            workers,
            self.scheduling.into(),
            |mft, chunk, options, reader, attr_reader| {
                let entries = mft.fold_rich_chunk(
                    chunk,
                    options,
                    reader,
                    attr_reader,
                    &|_| Vec::new(),
                    &|entries, entry| {
                        entries.push(entry);
                        Ok::<_, MftError>(())
                    },
                )?;
                map(RawMftChunkBatch { chunk, entries }).map_err(ScanError::Consumer)
            },
            |result| visit(result).map_err(ScanError::Consumer),
        )
    }
    /// Execute per-chunk folding without allocating intermediate batches,
    /// retaining application errors from either folding or ordered visitation.
    pub fn try_fold_chunks<E, Init, Fold, T, V>(
        self,
        init: Init,
        fold: Fold,
        mut visit: V,
    ) -> Result<(), ScanError<E>>
    where
        E: Send,
        Init: Fn(RawMftWorkChunk) -> T + Sync,
        Fold: Fn(&mut T, RawMftEntry) -> Result<(), E> + Sync,
        T: Send,
        V: FnMut(T) -> Result<(), E>,
    {
        let workers = self.resolved_worker_count()?;
        executor::run_parallel_chunks_in_order(
            self.mft,
            self.resolved_chunks()?,
            self.scan_options,
            workers,
            self.scheduling.into(),
            |mft, chunk, options, reader, attr_reader| {
                mft.fold_rich_chunk(chunk, options, reader, attr_reader, &init, &|acc, entry| {
                    fold(acc, entry).map_err(ScanError::Consumer)
                })
            },
            |result| visit(result).map_err(ScanError::Consumer),
        )
    }
}

impl<'m, 'v> RawMftParallelScan<'m, 'v, Lean> {
    /// Execute the scan and visit batches on the calling thread, preserving
    /// application errors as `ScanError::Consumer`. Returning an error stops
    /// delivery; already-running worker callbacks may finish before return.
    pub fn try_for_each_batch<E, F>(self, visit: F) -> Result<(), ScanError<E>>
    where
        E: Send,
        F: FnMut(RawMftChunkBatch<RawMftBatchEntry>) -> Result<(), E>,
    {
        self.try_map_chunks_for_each(Ok, visit)
    }
    /// Execute mapping on workers and visit mapped values in supplied chunk
    /// order. Both callback error types are preserved. This is a terminal
    /// operation, not a lazy iterator adapter.
    pub fn try_map_chunks_for_each<E, F, T, V>(
        self,
        map: F,
        mut visit: V,
    ) -> Result<(), ScanError<E>>
    where
        E: Send,
        F: Fn(RawMftChunkBatch<RawMftBatchEntry>) -> Result<T, E> + Sync,
        T: Send,
        V: FnMut(T) -> Result<(), E>,
    {
        let workers = self.resolved_worker_count()?;
        executor::run_parallel_chunks_in_order(
            self.mft,
            self.resolved_chunks()?,
            self.scan_options,
            workers,
            self.scheduling.into(),
            |mft, chunk, options, reader, attr_reader| {
                let entries =
                    mft.read_chunk_with_reused_readers(chunk, options, reader, attr_reader)?;
                map(RawMftChunkBatch { chunk, entries }).map_err(ScanError::Consumer)
            },
            |result| visit(result).map_err(ScanError::Consumer),
        )
    }
    /// Execute per-chunk folding without allocating intermediate batches,
    /// retaining application errors from either folding or ordered visitation.
    pub fn try_fold_chunks<E, Init, Fold, T, V>(
        self,
        init: Init,
        fold: Fold,
        mut visit: V,
    ) -> Result<(), ScanError<E>>
    where
        E: Send,
        Init: Fn(RawMftWorkChunk) -> T + Sync,
        Fold: Fn(&mut T, RawMftBatchEntry) -> Result<(), E> + Sync,
        T: Send,
        V: FnMut(T) -> Result<(), E>,
    {
        let workers = self.resolved_worker_count()?;
        executor::run_parallel_chunks_in_order(
            self.mft,
            self.resolved_chunks()?,
            self.scan_options,
            workers,
            self.scheduling.into(),
            |mft, chunk, options, reader, attr_reader| {
                mft.fold_chunk_with_reused_readers(
                    chunk,
                    options,
                    &init,
                    &|acc, entry| fold(acc, entry).map_err(ScanError::Consumer),
                    reader,
                    attr_reader,
                )
            },
            |result| visit(result).map_err(ScanError::Consumer),
        )
    }
}

impl RawMftParallelScan<'_, '_, Lean> {
    /// Execute deferred cached enrichment, preserving application callback
    /// errors. Like the library-error convenience method, this always uses
    /// dynamic scheduling and visits worker accumulators followed by the
    /// second-phase accumulator.
    pub fn try_fold_records_cached_enrich<E, Init, Fold, T, V>(
        self,
        init: Init,
        fold: Fold,
        mut visit: V,
    ) -> Result<(), ScanError<E>>
    where
        E: Send,
        Init: Fn() -> T + Sync,
        Fold: Fn(&mut T, RawMftBatchEntry) -> Result<(), E> + Sync,
        T: Send,
        V: FnMut(T) -> Result<(), E>,
    {
        let workers = self.resolved_worker_count()?;
        self.mft.fold_records_cached_enrich(
            self.resolved_chunks()?,
            self.scan_options,
            workers,
            init,
            |acc, entry| fold(acc, entry).map_err(ScanError::Consumer),
            |result| visit(result).map_err(ScanError::Consumer),
        )
    }
}
