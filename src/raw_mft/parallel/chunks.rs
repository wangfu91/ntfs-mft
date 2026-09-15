//! Chunk planning and chunk-local operations for parallel raw-MFT scans.

use std::num::NonZeroUsize;

use crate::{
    errors::MftError,
    raw_mft::{
        RawMft,
        attr_list::{enrich_batch_from_attr_list, should_enrich_batch_from_attr_list},
        chunk_plan::{self, RawMftChunkPlanOptions, RawMftWorkChunk},
        entry_build::{RawMftBatchEntry, RawMftBatchScratch, RawMftChunkBatch},
        options::RawMftScanOptions,
        serial::engine::{SerialParseState, next_record_output},
        volume_reader::VolumeReader,
    },
};

use super::ChunkScheduling;
use super::executor;

impl<'a> RawMft<'a> {
    /// Build deterministic logical work chunks for raw `$MFT` parsing.
    #[must_use]
    pub fn plan_chunks(&self) -> Vec<RawMftWorkChunk> {
        self.plan_chunks_with_options(
            RawMftChunkPlanOptions::default(),
            &RawMftScanOptions::default(),
        )
    }

    /// Build logical work chunks using scan selection and custom partition sizing.
    ///
    /// Pass the same scan options to execution. A precomputed plan cannot recover
    /// ranges or unused bands omitted by the options used to create it.
    #[must_use]
    pub fn plan_chunks_with_options(
        &self,
        options: RawMftChunkPlanOptions,
        scan_options: &RawMftScanOptions,
    ) -> Vec<RawMftWorkChunk> {
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

    /// Parse one logical work chunk into lean batch entries.
    ///
    /// This low-level operation uses the chunk range directly, ignoring the
    /// range in `options`. For automatic range intersection use [`Self::parallel`].
    pub fn read_chunk(
        &self,
        chunk: RawMftWorkChunk,
        options: RawMftScanOptions,
    ) -> Result<Vec<RawMftBatchEntry>, MftError> {
        let (mut reader, mut attr_reader) = self.buffered_readers_for_options(&options)?;
        self.read_chunk_with_reused_readers(chunk, &options, &mut reader, &mut attr_reader)
    }

    /// Parse one logical work chunk and fold lean batch entries into a caller-owned accumulator.
    ///
    /// As with [`Self::read_chunk`], the chunk supplies the range; the range in
    /// `options` is ignored.
    pub fn fold_chunk<T, Init, Fold>(
        &self,
        chunk: RawMftWorkChunk,
        options: RawMftScanOptions,
        init: &Init,
        fold_entry: &Fold,
    ) -> Result<T, MftError>
    where
        Init: Fn(RawMftWorkChunk) -> T,
        Fold: Fn(&mut T, RawMftBatchEntry) -> Result<(), MftError>,
    {
        let (mut reader, mut attr_reader) = self.buffered_readers_for_options(&options)?;
        self.fold_chunk_with_reused_readers(
            chunk,
            &options,
            init,
            fold_entry,
            &mut reader,
            &mut attr_reader,
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
        F: FnMut(RawMftBatchEntry) -> Result<(), E>,
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
    ) -> Result<Vec<RawMftBatchEntry>, MftError> {
        let mut entries = Vec::with_capacity(chunk.record_len().min(usize::MAX as u64) as usize);
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
        Fold: Fn(&mut T, RawMftBatchEntry) -> Result<(), E>,
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
