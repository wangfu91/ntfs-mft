# Choosing an MFT API

Use `RawMft` for a best-effort base-record listing from an NTFS volume.
Use `MftStream` for per-slot outcomes and metadata evidence from an extracted
logical MFT or a volume. Device access is read-only; live reads are not snapshots.

| API | Output | Selection and errors |
| --- | --- | --- |
| `raw.scan_entries()` | `RawMftEntry` | Starts at record 24; excludes bitmap-unused slots and extensions; skips malformed records; scan I/O errors end iteration |
| `raw.parallel().rich()` | Batches of `RawMftEntry` | Same raw defaults; scanner/callback errors end the operation |
| `raw.parallel().lean()` | Batches of `RawMftLeanEntry` | Same raw defaults with reduced metadata |
| `stream.iter_records()` | `MftResult<MftRecord>` per slot | Starts at zero, includes unused and extension records; parse errors are recoverable, I/O errors terminal |

## Selection and planning

Selection and metadata collection are separate options:

```rust,no_run
use ntfs_mft::{MftResult, RawMft, RawMftEntryOptions, RawMftScanOptions};

fn scan(raw: &RawMft<'_>) -> MftResult<()> {
    let metadata = RawMftEntryOptions::builder()
        .collect_alternate_data_streams(false)
        .build();
    let options = RawMftScanOptions::builder()
        .range(0..1000)
        .include_unused_records(true)
        .entry(metadata)
        .build();
    for entry in raw.scan_entries_with_options(options)? {
        println!("{}", entry?.file_name);
    }
    Ok(())
}
```

Scan options default to `24..`; `RawMftRecordRange::default()` represents `0..`.
The range builder accepts standard `u64` Rust ranges. Serial scans and rich
parallel planning/execution share `RawMftScanOptions`. Lean scans use
`RawMftLeanScanOptions`; `options.into_lean()` retains selection, buffers, and
DOS-link policy while discarding rich-only metadata settings.

| Setting | Default |
| --- | --- |
| Record range | `24..` |
| Bitmap-unused records | Excluded; unknown allocation remains eligible |
| ADS collection | Enabled for rich entries; unavailable for lean entries |
| Shadowed DOS 8.3 name links | Retained |
| Main read buffer | 256 KiB per reader |
| Attribute read buffer | 16 KiB per reader |
| Parallel workers | `thread::available_parallelism()`, capped by nonempty chunk count |
| Parallel scheduling | `Dynamic` |

`RawMftChunkPlanOptions` controls partition size, defaulting to 16,384 records
per chunk. Both `plan_chunks` and `plan_chunks_with_options` return
`MftResult<Vec<RawMftWorkChunk>>`. Automatic planning limits plans to 4,194,304
candidate chunks before bitmap filtering (64 MiB of descriptors); explicit
plans apply the limit to nonempty intersections after clipping.
Exceeding it returns `MftError::InvalidOptions`; allocation failure returns
`MftError::Io`. Use smaller selected ranges for larger scans. This cap does not
bound worker buffers, entry batches, caches, or collected output.

Explicit `.chunks(...)` are clipped to selection and volume bounds. Empty
intersections are dropped, overlaps rejected, and supplied order retained.
Batch and fold-initializer ranges are the clipped ranges. Unused-record
filtering still follows scan options; omitted chunks are not recreated.
Calling `.chunk_plan(...)` switches back to automatic planning, and builder
order does not change selection. For manual planning, pass the same scan
options to `plan_chunks_with_options` and execution.

`bitmap_usage(number)` returns `None` for unavailable or out-of-range evidence.
Unknown allocation is scanned conservatively; `entry.is_used` is the FILE-header state.

## Parallel callbacks

Choose `.rich()` or `.lean()` before configuring and executing a parallel scan:

```rust,no_run
use ntfs_mft::{MftResult, RawMft, RawMftScanOptions};

fn visit(raw: &RawMft<'_>) -> MftResult<()> {
    let options = RawMftScanOptions::builder().range(24..).build();
    raw.parallel().rich().scan_options(options).for_each_batch(|batch| {
        for entry in batch.entries {
            println!("{}: {}", entry.file_reference, entry.file_name);
        }
    })
}
```

Batch visitors run on the calling thread in chunk order. Map/fold callbacks run
on workers; a single worker executes chunks serially. For fallible callbacks,
use `try_*` methods and match `ScanError::Scanner(MftError)` or
`ScanError::Consumer(E)`. The consumer error must be `Send`. Already-running
worker callbacks may finish after another fails; completed side effects are
not rolled back.

### Cached enrichment

`raw.parallel().lean().try_fold_records_cached_enrich(init, fold, visit)`
captures extension records during scanning and enriches deferred base records
in a second phase. It always uses dynamic scheduling, ignoring `.scheduling(...)`.
It creates an accumulator per worker and one for the second phase, rather than
one per chunk. The calling thread visits worker accumulators in worker creation
order, followed by the second-phase accumulator after both phases succeed;
record and chunk order are not preserved.

The second-phase `fold` runs on the calling thread. Accumulators can be empty,
and callbacks should support merging independent partial results. Missing
dependencies can be read outside the selected chunks without adding output
records. Captured extensions and deferred records require additional memory.
Callback errors retain `ScanError::Consumer`; enrichment remains best effort.

## Metadata and errors

Rich entries include timestamps, attributes, and ADS subject to collection
options. Lean entries retain names, references, sizes, links, and basic flags;
they have no ADS toggle. Raw scans skip sparse and non-FILE slots; enrichment
failures are not returned to callers.
A successful scan or `entry.metadata()` does not establish complete metadata:
unobserved fields remain unknown.

`MftRecord` carries metadata states, diagnostics, and unavailable external
payloads. Preserve them alongside `entry` when completeness matters.
`MftStreamOptions::new(size)` disables extension resolution by default;
`.resolve_extensions(true)` follows one level of resident attribute-list references.
The [stream contract](offline-mft.md) explains partial results, terminal extension
I/O errors, and unavailable payloads.

Nonresident bitmap and attribute-list materialization is limited to 64 MiB per
payload. Oversized bootstrap payloads return an error; best-effort enrichment
leaves affected metadata unknown. Run coverage and offset arithmetic are checked
before allocation or I/O.

## Names and references

`NtfsName` retains exact UTF-16 across hosts. Use `as_utf16()` for identity/export,
`try_to_string()` for checked Unicode, and `to_string_lossy()` or `Display` for
readable text. `to_os_string()` is lossless on Windows; Unix native paths replace
unpaired surrogates.

`NtfsFileReference::try_from_parts(record, sequence)` rejects record-number
overflow. `base_reference()` is `None` for base records and `Some(full_reference)`
for extensions, including a base at record zero. Match record and sequence
numbers together: deleted slots can be reused.

Path reconstruction belongs to consumers. Include root record 5 and required
ancestors for a complete tree; default scans start at record 24. Handle missing
parents, reused slots, multiple hard links, and concurrent changes.

## Logical streams

`MftStream::from_path` and `from_reader` require an explicit record size.
`MftStream::from_raw_mft(&raw)` infers geometry and opens an independent handle,
so the stream can outlive the raw MFT and its volume. For custom options, use
`from_reader(raw.byte_reader()?, options)` with `raw.file_record_size()`.
See the [stream guide](offline-mft.md) for input requirements, byte-reader
behavior, metadata states, and extension handling.
