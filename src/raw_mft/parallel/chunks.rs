//! Chunk planning and chunk-local operations for parallel raw-MFT scans.

use std::num::NonZeroUsize;

use crate::{
    MftResult,
    errors::MftError,
    raw_mft::{
        RawMft,
        attr_list::{enrich_batch_from_attr_list, should_enrich_batch_from_attr_list},
        chunk_plan::{self, RawMftChunkPlanOptions, RawMftWorkChunk},
        entry_build::{RawMftBatchScratch, RawMftChunkBatch, RawMftLeanEntry},
        options::RawMftScanOptions,
        serial::engine::{SerialParseState, next_record_output},
        volume_reader::VolumeReader,
    },
};

use super::ChunkScheduling;
use super::executor;

impl<'a> RawMft<'a> {
    /// Build deterministic logical work chunks for raw `$MFT` parsing.
    ///
    /// # Errors
    /// Returns [`MftError::InvalidOptions`] when the candidate plan exceeds
    /// 64 MiB of chunk descriptors, or [`MftError::Io`] if allocation fails.
    /// Select smaller record ranges with [`Self::plan_chunks_with_options`].
    pub fn plan_chunks(&self) -> MftResult<Vec<RawMftWorkChunk>> {
        self.plan_chunks_with_options(
            RawMftChunkPlanOptions::default(),
            &RawMftScanOptions::default(),
        )
    }

    /// Build logical work chunks using scan selection and custom partition sizing.
    ///
    /// Pass the same scan options to execution. A precomputed plan cannot recover
    /// ranges or unused bands omitted by the options used to create it.
    ///
    /// # Errors
    /// Returns [`MftError::InvalidOptions`] if the selected range would require
    /// more than 4,194,304 candidate chunks (64 MiB), before bitmap filtering.
    /// Scan smaller record ranges to stay within this bound. Allocation failures
    /// return [`MftError::Io`]. This is not a limit on total scan-result memory.
    pub fn plan_chunks_with_options(
        &self,
        options: RawMftChunkPlanOptions,
        scan_options: &RawMftScanOptions,
    ) -> MftResult<Vec<RawMftWorkChunk>> {
        let range = scan_options.range();
        let end_record = range
            .end_record()
            .unwrap_or(self.record_count())
            .min(self.record_count());
        chunk_plan::build_work_chunks(
            range.start_record(),
            end_record,
            options.max_records_per_chunk(),
            scan_options.include_unused_records(),
            |record_number| self.bitmap_used(record_number),
        )
    }

    /// Visit each batch entry in a record-number range using caller-supplied
    /// readers so chunk workers can reuse their buffer state.
    fn for_each_batch_entry_in_range_with_readers<F, E>(
        &self,
        start_record: u64,
        end_record: u64,
        options: &RawMftScanOptions,
        reader: &mut VolumeReader,
        attr_reader: &mut VolumeReader,
        mut visit: F,
    ) -> Result<(), E>
    where
        E: From<MftError>,
        F: FnMut(RawMftLeanEntry) -> Result<(), E>,
    {
        let mut scan = SerialParseState::for_range(self, options, start_record, end_record);

        while let Some(entry) = next_record_output(self, &mut scan, reader, |record| {
            let record_number = record.number;

            let (mut entry, attr_list) = RawMftBatchScratch::from_record_with_attr_list(
                record,
                options.entry.collect_dos_file_name_links,
            );

            if let Some(attr_list) = attr_list
                && should_enrich_batch_from_attr_list(&entry)
            {
                let _ = enrich_batch_from_attr_list(
                    &mut entry,
                    attr_list,
                    record_number,
                    attr_reader,
                    &self.boot,
                    self.extent_map.as_ref(),
                    options.entry.collect_dos_file_name_links,
                );
            }

            Ok(entry.into_entry())
        })? {
            visit(entry)?;
        }

        Ok(())
    }

    /// Parse one chunk into batch entries using caller-supplied readers.
    pub(super) fn read_chunk_with_reused_readers(
        &self,
        chunk: RawMftWorkChunk,
        options: &RawMftScanOptions,
        reader: &mut VolumeReader,
        attr_reader: &mut VolumeReader,
    ) -> Result<Vec<RawMftLeanEntry>, MftError> {
        // Grow with parsed entries, never with an untrusted logical range size.
        let mut entries = Vec::new();
        self.for_each_batch_entry_in_range_with_readers(
            chunk.start_record,
            chunk.end_record,
            options,
            reader,
            attr_reader,
            |entry| {
                entries.push(entry);
                Ok::<_, MftError>(())
            },
        )?;
        Ok(entries)
    }

    /// Fold one chunk into a caller-owned accumulator using caller-supplied readers.
    pub(super) fn fold_chunk_with_reused_readers<T, Init, Fold, E>(
        &self,
        chunk: RawMftWorkChunk,
        options: &RawMftScanOptions,
        init: &Init,
        fold_entry: &Fold,
        reader: &mut VolumeReader,
        attr_reader: &mut VolumeReader,
    ) -> Result<T, E>
    where
        E: From<MftError>,
        Init: Fn(RawMftWorkChunk) -> T,
        Fold: Fn(&mut T, RawMftLeanEntry) -> Result<(), E>,
    {
        let mut acc = init(chunk);
        self.for_each_batch_entry_in_range_with_readers(
            chunk.start_record,
            chunk.end_record,
            options,
            reader,
            attr_reader,
            |entry| fold_entry(&mut acc, entry),
        )?;
        Ok(acc)
    }

    /// Parse logical work chunks in parallel, transform them on worker threads, and visit results
    /// in deterministic chunk order.
    pub(crate) fn for_each_mapped_chunk<F, T, V>(
        &self,
        chunks: Vec<RawMftWorkChunk>,
        options: RawMftScanOptions,
        worker_count: NonZeroUsize,
        scheduling: ChunkScheduling,
        map_chunk: F,
        visit: V,
    ) -> Result<(), MftError>
    where
        F: Fn(RawMftChunkBatch) -> Result<T, MftError> + Sync,
        T: Send,
        V: FnMut(T) -> Result<(), MftError>,
    {
        executor::run_parallel_chunks_in_order(
            self,
            chunks,
            options,
            worker_count,
            scheduling,
            move |mft, chunk, options, reader, attr_reader| {
                mft.read_chunk_with_reused_readers(chunk, options, reader, attr_reader)
                    .and_then(|entries| map_chunk(RawMftChunkBatch { chunk, entries }))
            },
            visit,
        )
    }

    /// Parse logical work chunks in parallel and visit batches in deterministic order.
    pub(crate) fn for_each_chunk<F>(
        &self,
        chunks: Vec<RawMftWorkChunk>,
        options: RawMftScanOptions,
        worker_count: NonZeroUsize,
        scheduling: ChunkScheduling,
        visit: F,
    ) -> Result<(), MftError>
    where
        F: FnMut(RawMftChunkBatch) -> Result<(), MftError>,
    {
        self.for_each_mapped_chunk(
            chunks,
            options,
            worker_count,
            scheduling,
            Ok::<_, MftError>,
            visit,
        )
    }

    /// Parse logical work chunks in parallel using worker-local readers and custom options.
    pub(crate) fn read_chunks(
        &self,
        chunks: Vec<RawMftWorkChunk>,
        options: RawMftScanOptions,
        worker_count: NonZeroUsize,
        scheduling: ChunkScheduling,
    ) -> Result<Vec<RawMftChunkBatch>, MftError> {
        let mut ordered_batches = Vec::with_capacity(chunks.len());
        self.for_each_chunk(chunks, options, worker_count, scheduling, |batch| {
            ordered_batches.push(batch);
            Ok(())
        })?;
        Ok(ordered_batches)
    }
}

impl RawMft<'_> {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn fold_rich_chunk<T, Init, Fold, E>(
        &self,
        chunk: RawMftWorkChunk,
        options: &RawMftScanOptions,
        reader: &mut VolumeReader,
        attr_reader: &mut VolumeReader,
        init: &Init,
        fold: &Fold,
    ) -> Result<T, E>
    where
        E: From<MftError>,
        Init: Fn(RawMftWorkChunk) -> T,
        Fold: Fn(&mut T, crate::RawMftEntry) -> Result<(), E>,
    {
        let mut acc = init(chunk);
        let build_options = super::super::reader::entry_build_options(options);
        let mut scan =
            SerialParseState::for_range(self, options, chunk.start_record, chunk.end_record);
        while let Some(entry) = next_record_output(self, &mut scan, reader, |record| {
            Ok(super::super::reader::build_rich_entry(
                self,
                record,
                attr_reader,
                build_options,
            ))
        })? {
            fold(&mut acc, entry)?;
        }
        Ok(acc)
    }
}
