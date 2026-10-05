# Live volume scan internals

`RawMft` reads NTFS boot metadata and `$MFT` directly from a volume or device.
It does not use Windows FSCTL enumeration. ReFS is rejected with
`MftError::UnsupportedFilesystem`. All volume access is read-only; see
[Windows](windows-raw-volume-access.md) and [macOS](macos-raw-volume-access.md)
access requirements. Start with the [API guide](api-guide.md) for public entry
points and the [stream contract](offline-mft.md) for `MftStream` parsing.

## Discovery and I/O

### What the MFT contains

The Master File Table is NTFS's record table for files and directories, including
the filesystem's own metadata files. Each file or directory has at least one
record; attributes that do not fit can occupy extension records. NTFS reserves
the first 16 record numbers for system use. `$MFTMirr` duplicates an initial
portion of the MFT for recovery, rather than the entire table.

A record is commonly 1 KiB, but its size comes from the volume's boot geometry.
The record header describes its identity, flags, update sequence array, and
attribute area. Attributes then describe the file:

| Component | Purpose |
| --- | --- |
| FILE record header | Record identity, sequence number, layout, and fixup information |
| `$STANDARD_INFORMATION` | Timestamps, flags, and other standard metadata |
| `$FILE_NAME` | A name, namespace, parent reference, and duplicated metadata; hard links can add names |
| Security information | A security descriptor or an identifier referring to shared security metadata |
| `$DATA` | File contents or a runlist locating nonresident content; named streams represent alternate data streams |
| Other attributes | Directory indexes, attribute lists, reparse information, and other NTFS metadata |

Headers and attributes have format-dependent, variable sizes; they are not fixed
42-byte or 70-byte blocks. Small attribute values, including complete small-file
contents, can reside inside the MFT record itself. This means an extracted MFT
can contain file contents as well as names and timestamps.

`RawMft::new` reads the boot sector and FILE record zero, decodes the unnamed
`$MFT::$DATA` runlist into an `ExtentMap`, and loads `$MFT::$BITMAP` into memory.
Boot geometry supplies sector size, cluster size, record size, and MFT location.
Record size comes from that geometry; it is not fixed at 1 KiB.

`ExtentMap` translates logical record numbers through fragmented physical
extents and sparse holes. `record_location` caches the segment index and checks
complete record coverage. Chunk boundaries remain logical record ranges, not
physical extent boundaries.

`VolumeReader` buffers aligned reads. Windows uses explicit-offset `ReadFile`
with a per-reader completion event; Linux and macOS use positional reads.
macOS also queries device block alignment.
The normal scan borrows contiguous record bytes from the buffer, avoiding a
per-record copy; records spanning physical extents use a reusable scratch buffer.
A separate attribute reader handles random extension lookups without disturbing
the sequential buffer.

`Volume` retains its source for reopening: a Windows drive letter, a mount point,
or a Linux/macOS device path. Linux resolves mounts through `/proc/self/mountinfo`;
macOS uses `statfs`. Workers and owned byte streams reopen independent read-only
handles. A live scan is not an atomic snapshot.

## Serial scanning

`scan_entries` and `scan_entries_with_options` use the following pipeline:

1. Check the cached bitmap when unused records are excluded.
2. Translate the record number through the extent map; skip sparse holes.
3. Read the FILE record, validate its header, and apply USA fixups in the buffer.
4. Reject extension records (`base_reference != 0`) before building metadata.
5. Build a `RawMftEntry` and enrich missing metadata from its attribute list.

Default scans start at record 24 and exclude unused records. Invalid records
are skipped; parse failures are logged. Scan I/O errors are returned once and
terminate iteration.
`bitmap_usage` returns `None` when allocation evidence is unavailable; those
slots remain scan candidates. Inspect `entry.is_used` for the FILE-header state.
This output is a best-effort base-record view, not a guaranteed Explorer listing.
Use [MftStream](offline-mft.md) to inspect every logical record position and its errors.

## Metadata and extension records

The entry builder collects standard information, names, data streams, reparse
information, and attribute-list references. Name selection prefers
`Win32AndDos`, then `Win32`, `Posix`, and `Dos`. Collection options control
shadowed DOS aliases and alternate streams. Unnamed DATA
supplies primary size/storage metadata; named DATA supplies alternate streams.

Overflow attributes live in extension records referenced by `$ATTRIBUTE_LIST`.
Enrichment runs when the base entry needs a better name, missing file data, or a
missing reparse tag. It reads resident or nonresident lists, loads the required
extension records, and merges usable metadata. It follows only one level and
does not yield extensions as separate files. Raw iteration discards enrichment
errors; consumers requiring completeness diagnostics should use `MftStream`.

Rich serial and parallel scans share metadata construction and enrichment.
Lean parallel scans keep names, references, sizes, links, and basic flags; their
options omit ADS settings. `.into_lean()` explicitly discards rich-only choices.
Cached enrichment is lean-only.

`RawMftEntry::metadata()` reports observed presence and disabled collection;
it leaves completeness unverified and missing fields unknown. It does not
reparse attributes or allocate diagnostics. For per-slot parsing evidence, use
`MftStream::from_raw_mft(raw)` or construct a stream from `raw.byte_reader()`.
Retain each record's `metadata`, `diagnostics`, and `external_payloads`.

File and parent references preserve a 48-bit record number and a 16-bit sequence
number. `base_reference()` is `None` for base records and `Some(full_reference)`
for extensions, including extensions of record zero. Consumers that reconstruct
paths must match full references and handle missing ancestors, reused slots,
cycles, and multiple hard links. The library does not construct paths or trees.

## Chunk planning

`RawMftScanOptions` is the single source of record selection for automatic
parallel planning and scanning. Its defaults select record 24 through
`record_count()` and exclude unused records. `RawMftChunkPlanOptions` controls
only partition size, defaulting to at most 16,384 logical records per chunk.
The planner keeps dense bounded bands and omits bands with no used records
unless the scan options include unused records.

Planning checks a limit of 4,194,304 candidate chunks before bitmap filtering,
equivalent to 64 MiB of chunk descriptors. A plan exceeding that limit returns
`MftError::InvalidOptions`; a failed planning allocation returns `MftError::Io`.
For very large scans, process smaller selected record ranges separately. The
limit covers planning descriptors, not total scan memory: worker buffers,
entries, and collected results consume additional memory.

```rust,no_run
use ntfs_mft::{MftResult, RawMft, RawMftChunkPlanOptions, RawMftScanOptions};
use std::num::NonZeroU64;

fn scan(raw: &RawMft<'_>, records_per_chunk: NonZeroU64) -> MftResult<()> {
    let options = RawMftScanOptions::builder()
        .range(0..1000)
        .include_unused_records(true)
        .build();
    raw.parallel().rich()
        .scan_options(options)
        .chunk_plan(RawMftChunkPlanOptions::builder()
            .max_records_per_chunk(records_per_chunk)
            .build())
        .for_each_batch(|batch| {
            println!("{} entries", batch.entries.len());
        })
}
```

Builder order does not change selection. Explicit `.chunks(...)` are candidate
ranges: execution intersects each with the scan range and volume bounds, drops
empty intersections, and preserves supplied order. Overlapping intersections
are rejected with `MftError::InvalidOptions` before scanning; adjacent ranges
are valid. The same chunk-count limit applies to nonempty intersections after
clipping. Returned batch ranges and fold initializer ranges
are the clipped ranges. Unused-record filtering still uses the scan options.
Omitted chunks are never recreated, even when unused records are enabled.
Calling `.chunk_plan(...)` switches back to automatic planning.

For manual planning, call `raw.plan_chunks_with_options(chunk_options, &scan_options)?`
and pass the same scan options to parallel execution. Prefer automatic planning
when the selection may change. For sequential chunk execution, configure
`.parallel().lean().workers(NonZeroUsize::MIN)`;
manual chunks use the same selection contract as multi-worker execution.

## Parallel execution

`RawMft::parallel()` requires `.rich()` or `.lean()` before execution. Both provide
`collect_batches`, `for_each_batch`, `try_for_each_batch`, `try_map_chunks_for_each`,
and `try_fold_chunks`. One worker falls back to serial chunk processing.

Each worker reopens the source and owns sequential and attribute readers.
Boot geometry is cloned; the extent map and bitmap are shared through `Arc`.
Workers perform both I/O and parsing using the common scan engine.

- `Dynamic`: workers claim the next chunk from a shared atomic index.
- `Contiguous`: each worker receives a fixed band of chunk indices.

Results may finish out of order. The coordinator buffers them and invokes the
visitor in the supplied chunk order. Worker errors, unexpected channel closure,
and worker panics fail the operation.

`try_map_chunks_for_each` transforms each parsed batch on its worker.
`try_fold_chunks` initializes an accumulator per chunk and folds entries into
it, avoiding an intermediate batch vector. Both preserve ordered result delivery.

### Application callback errors

Both projections provide `try_for_each_batch`, `try_map_chunks_for_each`, and
`try_fold_chunks`. Lean scans also provide `try_fold_records_cached_enrich`.
That method uses captured extension records when available and reads missing
extensions from the volume, including those outside the selected range or
explicit chunks. These dependency reads do not add output records.
These are terminal operations returning `Result<_, ScanError<E>>`:
`Scanner(MftError)` preserves scanner failures and `Consumer(E)` preserves the
application error. `E` must be `Send` because worker callbacks can fail.
Annotate a callback's return type when inference cannot determine `E`.

For callbacks that cannot fail, use `for_each_batch`, returning `MftResult<()>`.
The `try_*` methods preserve fallible application callbacks. Returning a
callback error stops further result delivery, but
already-running worker callbacks may finish before the call returns. No
rollback of prior callback side effects is implied. Mapping and folding run
on workers; ordered visitation runs on the calling thread.

## Benchmarks and tests

`benches/raw_mft_ingest.rs` and its profile example share
`support/raw_mft_ingest_support.rs`. The benchmark disables ADS and DOS aliases;
both planning and scanning exclude unused records. Its defaults
are 2,048-record chunks, 256 KiB/16 KiB buffers, and available parallelism capped
at 10 workers. These are workload-specific benchmark settings.

See [parallel measurements and commands](raw-mft-parallel-ingest-findings.md)
and [serial measurements](raw-mft-serial-read-findings.md). The
[sparse-volume fixture](../tests/fixtures/sparse-volume-fixture/README.md) tests
bootstrapping, fragmented bitmap loading, and serial/parallel agreement without
a device. The [module map](../src/raw_mft/README.md) locates the implementation.
