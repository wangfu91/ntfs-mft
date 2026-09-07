# Raw MFT reader internals

`RawMft` reads NTFS boot metadata and `$MFT` directly. It does not use Windows
FSCTL enumeration. ReFS is rejected with `MftError::UnsupportedFilesystem`.
All volume access is read-only; see [Windows](windows-raw-volume-access.md) and
[macOS](macos-raw-volume-access.md) access requirements.

For public entry points and their different contracts, start with
[Choosing an MFT API](api-guide.md).

## Discovery and I/O

`RawMft::new` reads the boot sector and FILE record zero, decodes the unnamed
`$MFT::$DATA` runlist into an `ExtentMap`, and loads `$MFT::$BITMAP` into memory.
Boot geometry supplies sector size, cluster size, record size, and MFT location.
Record size comes from that geometry; it is not fixed at 1 KiB.

`ExtentMap` translates logical record numbers through fragmented physical
extents and sparse holes. `record_offset_with_cursor` caches the segment index
for sequential access. Chunk boundaries remain logical record ranges, not
physical extent boundaries.

`VolumeReader` buffers aligned reads. Windows uses `SetFilePointerEx`/`ReadFile`;
Linux and macOS use positional reads. macOS also queries device block alignment.
The normal scan borrows record bytes from the buffer, avoiding a per-record copy.
A separate attribute reader handles random extension lookups without disturbing
the sequential buffer.

`Volume` retains its source for reopening: a Windows drive letter, a mount point,
or a Linux/macOS device path. Linux resolves mounts through `/proc/self/mountinfo`;
macOS uses `statfs`. Workers and owned byte streams reopen independent read-only
handles. A live scan is not an atomic snapshot.

## Serial scanning

`try_iter` and `try_iter_with_options` use the following pipeline:

1. Check the cached bitmap when unused records are excluded.
2. Translate the record number through the extent map; skip sparse holes.
3. Read the FILE record, validate its header, and apply USA fixups in the buffer.
4. Reject extension records (`base_reference != 0`) before building metadata.
5. Build a `RawMftEntry` and enrich missing metadata from its attribute list.

Default scans start at record 24 and exclude unused records. Invalid records
are logged and skipped; scan I/O errors are returned. If no bitmap is available,
`bitmap_used` returns true and callers must inspect `entry.is_used` themselves.
This output is a best-effort base-record view, not a guaranteed Explorer listing.
Use [MftStream](offline-mft.md) to inspect every physical slot and its errors.

## Metadata and extension records

The entry builder collects standard information, names, data streams, reparse
information, and attribute-list references. Name selection prefers
`Win32AndDos`, then `Win32`, `Posix`, and `Dos`. Collection options control
shadowed DOS aliases, alternate streams, and data-run summaries. Unnamed DATA
supplies primary size/storage metadata; named DATA supplies alternate streams.

Overflow attributes live in extension records referenced by `$ATTRIBUTE_LIST`.
Enrichment runs when the base entry needs a better name, missing file data, or a
missing reparse tag. It reads resident or nonresident lists, loads the required
extension records, and merges usable metadata. It follows only one level and
does not yield extensions as separate files. Raw iteration discards enrichment
errors; consumers requiring completeness diagnostics should use the offline API.

File references combine a 48-bit record number and a 16-bit sequence number.
`MftPathIndex::resolve_strict` verifies full references, including parent and root
sequence numbers. Reused slots return a structured mismatch error. The explicit
`resolve_best_effort` method reports exact, record-fallback, or partial quality;
partial paths are suffixes, not paths relative to the volume root. Cycles,
missing parents, and excessive depth are reported.

Build `MftPathIndex` during an existing scan with `insert` (rich or offline
entries) or `insert_lean`, retaining root record 5 and all needed parents. It
requires no volume and performs no I/O. Attach a volume prefix afterward with
`RawMftPathResolver::from_index`. The convenience `mft.path_resolver()` performs
a full additional scan, propagates scan I/O errors, and uses memory proportional
to records and names. Neither construction path makes a live volume atomic.
Raw scanners can still skip malformed records, so successful construction does
not establish that the index covers every file.

## Chunk planning

`RawMftScanOptions` is the single source of record selection for automatic
parallel planning and scanning. Its defaults select record 24 through
`record_count()` and exclude unused records. `RawMftChunkPlanOptions` controls
only partition size, defaulting to at most 16,384 logical records per chunk.
The planner keeps dense bounded bands and omits bands with no used records
unless the scan options include unused records.

```rust,no_run
use ntfs_mft::{MftResult, RawMft, RawMftChunkPlanOptions, RawMftScanOptions};
use std::num::NonZeroU64;

fn scan(raw: &RawMft<'_>) -> Result<(), ntfs_mft::ScanError<std::convert::Infallible>> {
    let options = RawMftScanOptions::builder()
        .start_record(0)
        .end_record(Some(1000))
        .include_unused_records(true)
        .build();
    raw.parallel().rich()
        .scan_options(options)
        .chunk_plan(RawMftChunkPlanOptions::builder()
            .max_records_per_chunk(NonZeroU64::new(256).unwrap())
            .build())
        .try_for_each_batch(|batch| {
            println!("{} entries", batch.entries.len());
            Ok(())
        })
}
```

Builder order does not change selection. Explicit `.chunks(...)` are candidate
ranges: execution intersects each with the scan range and volume bounds, drops
empty intersections, and preserves supplied order. Overlapping intersections
are rejected with `MftError::InvalidOptions` before scanning; adjacent ranges
are valid. Returned batch ranges and fold initializer ranges
are the clipped ranges. Unused-record filtering still uses the scan options.
Omitted chunks are never recreated, even when unused records are enabled.
Calling `.chunk_plan(...)` switches back to automatic planning.

For manual planning, call `raw.plan_chunks_with_options(chunk_options, &scan_options)`
and pass the same scan options to parallel execution. Prefer automatic planning
when the selection may change. The low-level `read_chunk` and `fold_chunk`
methods take their range directly from the supplied chunk; their scan-option
range is not applied.

### Migrating selection from chunk-plan options

Move `range`, `start_record`, `end_record`, and `include_unused_records` from
`RawMftChunkPlanOptions::builder()` to `RawMftScanOptions::builder()` and pass
the resulting options to `.scan_options(...)`. These selection methods and
getters have been removed from chunk-plan options. Manual planning now also
requires a borrowed scan-options argument. Explicit parallel chunks no longer
override the scan range: set `start_record(0)` when including system records.
Overlapping effective chunks now return an error; provide disjoint ranges.

## Parallel execution

`RawMft::parallel()` requires `.rich()` or `.lean()` before execution. Both provide `collect_batches`, `try_for_each_batch`, `try_map_chunks_for_each`,
and `try_fold_chunks`. One worker falls back to serial chunk processing.

Each worker reopens the source and owns sequential and attribute readers.
Boot geometry is cloned; the extent map and bitmap are shared through `Arc`.
Workers perform both I/O and parsing using the common scan engine.

- `Dynamic`: workers claim the next chunk from a shared atomic index.
- `Contiguous`: each worker receives a fixed band of chunk indices.

Results may finish out of order. The coordinator buffers them and invokes the
visitor in the supplied chunk order. Worker errors, unexpected channel closure,
and worker panics fail the operation.

`try_map_chunks_for_each` transforms each parsed batch on its worker. `try_fold_chunks` initializes
an accumulator per chunk and folds entries into it, avoiding an intermediate
batch vector. Both preserve ordered result delivery.

## Benchmarks and tests

`benches/raw_mft_ingest.rs` and its profile example share
`support/raw_mft_ingest_support.rs`. The benchmark disables ADS, run summaries,
and DOS aliases; both planning and scanning exclude unused records. Its defaults
are 2,048-record chunks, 256 KiB/16 KiB buffers, and available parallelism capped
at 10 workers. These are workload-specific benchmark settings.

See [parallel measurements and commands](raw_mft_parallel_ingest_findings.md)
and [serial measurements](raw_mft_serial_read_findings.md). The
[ESD-ISO fixture](../tests/fixtures/esd-iso/README.md) tests bootstrapping,
fragmented bitmap loading, and serial/parallel agreement without a device.
The [module map](../src/raw_mft/README.md) locates the implementation.

## Metadata projection migration

Choose `.parallel().rich()` for the full `RawMftEntry` shape used by serial
iteration, or `.parallel().lean()` for `RawMftBatchEntry`. Rich scans honor ADS,
data-run-summary, and DOS-link collection settings. Lean scans accept
`RawMftLeanScanOptions`, which exposes only selection, buffers, and DOS-link
collection. Both entry shapes carry `is_used` from the FILE header.

Existing parallel consumers can preserve their lean output by adding `.lean()`
and using `RawMftLeanScanOptions::builder()`. When adapting an existing rich scan
configuration intentionally, `.into_lean()` retains selection, buffers, and
DOS-link policy while discarding rich-only collection choices. Cached enrichment
remains a lean-only execution method. Rich parallel scans use the same inline
materialization and enrichment implementation as rich serial scans.

## Diagnostic raw scanning

The fast raw APIs remain tolerant. For evidence about skipped records and
metadata, use `raw.try_iter_diagnostic(options, diagnostic_limit)`. It yields
`RawMftDiagnosticRecord` values with per-category `MftFieldState` evidence and
metadata diagnostics. Base attributes receive the offline parser's validation;
attribute-list enrichment is conservatively reported as unverified because the
raw enrichment path does not inspect the complete extension attribute set.

```rust,no_run
use ntfs_mft::{RawMft, RawMftScanOptions, MftResult};
fn inspect(raw: &RawMft<'_>) -> MftResult<()> {
    let mut records = raw.try_iter_diagnostic(RawMftScanOptions::default(), 100)?;
    for result in records.by_ref() {
        let record = result?;
        println!("{} {:?}", record.entry, record.metadata);
    }
    let report = records.into_report();
    println!("{} yielded; {} invalid; reached end: {}",
        report.yielded, report.invalid_records, report.reached_end);
    Ok(())
}
```

Recoverable parse failures consume a slot and scanning continues. A scan I/O
error is returned once and stops the diagnostic iterator. The report distinguishes
unused, sparse, non-FILE/header-invalid, extension, and parser-rejected slots.
The diagnostic limit bounds retained parser errors, not their counts. Early
stopping leaves `reached_end` false, and reaching the end does not imply complete
metadata. Per-record metadata diagnostics are bounded by the physical record.
Diagnostic scans are currently serial; ordinary rich/lean parallel scans retain
the fast best-effort policy.

Every rich entry now retains its `collection_options`. Disabled ADS collection
is `NotCollected` in diagnostic metadata, rather than evidence of absence.
Ordinary empty fields still do not establish absence. For JSONL output, use
`JsonlWriter::write_diagnostic` to retain the record's evidence; `write_entry`
only exports the fast raw entry representation.

## Application callback errors

Both projections provide `try_for_each_batch`, `try_map_chunks_for_each`, and
`try_fold_chunks`. Lean scans also provide `try_fold_records_cached_enrich`.
These are terminal operations returning `Result<_, ScanError<E>>`:
`Scanner(MftError)` preserves scanner failures and `Consumer(E)` preserves the
application error. `E` must be `Send` because worker callbacks can fail.
Annotate a callback's return type when inference cannot determine `E`.

The `try_*` methods are the only fallible callback entry points. Returning a
callback error stops further result delivery, but
already-running worker callbacks may finish before the call returns. No
rollback of prior callback side effects is implied. Mapping and folding run
on workers; ordered visitation runs on the calling thread.

## NTFS reference migration

Raw rich/lean entries, name links, and path indexes now use `NtfsFileReference`
for file and parent references. `record_number()` returns `u64`, `sequence()`
returns `u16`, and `get()` returns the packed `u64`, without optional branches.
`try_from_parts(record_number, sequence)` rejects numbers above 48 bits instead
of silently truncating them. `new(raw)` preserves all bits of an existing NTFS
reference. Convert to legacy generic `Fid` with `.into()`, or back with
`NtfsFileReference::try_from(fid)`; extended IDs are rejected even if their
numeric value fits in 64 bits.

The old `base_record_reference` field was already a truncated record number;
it is now named `base_record_number` to state that meaning. Its zero value does
not distinguish a base record from an extension of record zero. The versioned
export DTO retains its existing `base_record_number` wire representation.

## Standard record ranges

Rich and lean option builders accept `.range(100..200)`, `.range(24..)`,
`.range(..)`, and inclusive forms such as `.range(..=100)`. A standalone
`RawMftRecordRange::default()` now means `0..`; scan-option defaults still mean
`24..`, preserving the ordinary scan policy. Empty ranges remain empty, and an
inclusive `u64::MAX` upper bound becomes an unbounded end clipped to the MFT.

### Lossless NTFS names

`RawMftEntry::file_name`, `RawMftBatchEntry::file_name`, hard-link names, and ADS
names use `NtfsName`. Parsing and extension merging preserve every original
UTF-16 code unit on Windows, Linux, and macOS. Equality and hashing use those
units without case folding or normalization.

- `name.as_utf16()` borrows the exact original units; `into_utf16()` takes ownership.
- `name.try_to_string()` rejects unpaired surrogates.
- `name.to_string_lossy()` and `Display` provide readable text with replacement characters.
- `name.to_os_string()` preserves units on Windows and replaces unpaired surrogates
  on Unix. Native paths produced by path resolvers have this same limitation;
  their quality describes reference matching, not lossless name conversion.

Use `NtfsName::from("file.txt")` for Unicode text or
`NtfsName::from_utf16(vec![0x61, 0xd800])` for exact code units. Names are stored
once as UTF-16, without a second cached native string. `MftPathIndex::insert_reference`
accepts `&NtfsName`; it validates path components separately. Export uses the
existing `windows_utf16` encoding tag for NTFS names on every platform and reports
`name_fidelity: "ntfs_utf16_preserved"`.
