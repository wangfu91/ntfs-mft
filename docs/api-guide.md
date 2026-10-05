# Choosing an MFT API

Use `RawMft` for a best-effort base-record listing from an NTFS volume. Use
`MftStream` for per-slot outcomes and metadata evidence, from either an extracted
logical `$MFT` or a live volume. All volume access is read-only; live reads are
not atomic filesystem snapshots.

| Entry point | Result | Default selection | Error behavior |
| --- | --- | --- | --- |
| `raw.scan_entries()` | `RawMftEntry` | Records `24..`, excludes bitmap-unused slots, skips extensions | Invalid records skipped; scan I/O errors returned |
| `raw.parallel().rich()` | Batches of `RawMftEntry` | Same raw defaults | Same tolerant parsing; scanner/callback errors end the operation |
| `raw.parallel().lean()` | Batches of `RawMftLeanEntry` | Same raw defaults | Same tolerant parsing; scanner/callback errors end the operation |
| `stream.iter_records()` | `Result<MftRecord, MftError>` per slot | All positions from zero, including unused and extensions | Parse errors returned per position and iteration continues; I/O error stops iteration |

## Errors and completeness

Raw scans skip sparse, non-FILE, malformed, and extension records. Enrichment
failures are not returned, so a successful scan does not prove complete metadata.
`bitmap_usage(number)` returns `None` for unavailable or out-of-range allocation
evidence. Unknown allocation is scanned conservatively; inspect each entry's
`is_used` for the FILE-header state.

`MftStream` returns record-validation errors per slot and preserves partial
metadata with diagnostics. When an extension lookup fails with an I/O error, the
iterator yields the partial base record, then `MftError::ExtensionRead`, then
stops. `collect::<MftResult<Vec<_>>>()` reports that I/O failure as an error;
successful collection still requires checking each record's metadata and
diagnostics for recoverable limitations. A direct `read_record` call retains
extension failures in the returned record's diagnostics.

Nonresident metadata materialization (`$MFT::$BITMAP` and `$ATTRIBUTE_LIST`)
is limited to 64 MiB per payload. Oversized payloads return an error during
bootstrap; best-effort entry enrichment leaves the affected metadata unknown.
Run coverage and offset arithmetic are checked before allocation or I/O.

## Selection and metadata are separate choices

```rust
use ntfs_mft::{RawMftEntryOptions, RawMftScanOptions};

let metadata = RawMftEntryOptions::builder()
    .collect_alternate_data_streams(false)
    .build();
let options = RawMftScanOptions::builder()
    .range(0..1000)
    .include_unused_records(true)
    .entry(metadata)
    .build();
```

Live volume scan options default to `24..`; standalone `RawMftRecordRange::default()`
represents `0..`. All standard `u64` Rust ranges are accepted by `.range()`.
The same rich options govern serial scanning and both planning and execution
of rich parallel scans. `RawMftChunkPlanOptions` controls partition size only.
Explicit chunks are clipped to selection and volume bounds; overlapping chunks
are rejected. Results are delivered in supplied chunk order.

Chunk planning is fallible. `plan_chunks_with_options` returns
`MftResult<Vec<RawMftWorkChunk>>`; automatic parallel planning propagates the same
errors. Plans are limited to 4,194,304 candidate chunks before bitmap filtering
(64 MiB of chunk descriptors). Explicit plans apply this limit to nonempty
intersections after clipping. Exceeding the limit returns
`MftError::InvalidOptions`; planning allocation failures return `MftError::Io`.
Split large scans into smaller selected record ranges. This limit does not bound
total scan memory, including entry batches, worker buffers, or collected output.

Rich entries include timestamps, attributes, and ADS, subject to
`collection_options`. `entry.metadata()` reports observed fields conservatively,
without another parse or diagnostic allocations: unobserved fields stay unknown.
Lean entries keep names, references, sizes, links, and basic flags; their options
expose no ADS toggle.
`options.into_lean()` explicitly retains selection/buffers/DOS-link policy and
discards the rich-only metadata choices.

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

Choose `.rich()` or `.lean()` before executing a parallel scan. For fallible
callbacks, use the `try_*` methods and match `ScanError::Scanner` or
`ScanError::Consumer`. Worker callbacks already in progress may finish after
another callback fails; completed side effects are not rolled back. Batch
visitors run on the calling thread; map and fold callbacks run on workers.

## Preserve evidence, names, and identity

Use `MftRecord.metadata` to distinguish absent, present, and unknown fields, and
retain its diagnostics when completeness matters. Raw attribute-list enrichment
remains selective; use `MftStream` inspection for its supported
extension-validation evidence.
`MftStreamOptions::new(size)` defaults to no extension resolution; enable
`.resolve_extensions(true)` for one level of resident attribute-list references.
An MFT-only source cannot recover arbitrary nonresident attribute payloads.

`NtfsName` retains exact UTF-16 for selected names, links, and ADS on every host.
Use `as_utf16()` for identity/export, `try_to_string()` for checked Unicode,
and `to_string_lossy()` or `Display` for readable text. `to_os_string()` is
lossless on Windows; Unix native paths replace unpaired surrogates.

`NtfsFileReference::try_from_parts(record, sequence)` rejects record-number
overflow. Its record and sequence accessors return values directly.
`base_reference()` returns `Option<NtfsFileReference>`, preserving the identity of an
extension record's base, including record zero. A record number alone
is not durable identity because deleted slots can be reused.

Full-path reconstruction belongs to the caller. Build your application's tree
from the returned names, namespaces, file references, parent references, and
hard-link metadata. Include root record 5 and required ancestors when scanning
for a complete tree; default live scans start at record 24. Match full references,
including sequence numbers, and handle missing parents and concurrent changes.

## Parsing a logical stream

`MftStream::from_path(path, options)` and `from_reader(reader, options)` require
an explicit `usize` record size. If you already have a `RawMft`, use
`MftStream::from_raw_mft(&raw)` to infer geometry and open an independent handle.
The resulting stream can outlive `raw` and its volume. For custom options, use
`from_reader(raw.byte_reader()?, options)` with `raw.file_record_size()`.

See [raw scan details](raw-mft-read.md) and [MFT stream parsing](offline-mft.md) for their full contracts.
