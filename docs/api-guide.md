# Choosing an MFT API

Use a live volume scan for a best-effort base-record listing from a volume. Use
`MftStream` when each logical record position, including invalid or extension
records, needs an explicit outcome or metadata evidence.

A **live volume scan** reads directly from an NTFS volume/device. An **offline
MFT scan** parses an extracted logical `$MFT` file or buffer. `read` denotes a
single-record or byte operation, `scan` denotes traversal, and `parse` denotes
record interpretation. `RawMft` and `MftStream` name the data access abstractions;
`MftStream` can parse either source, so using it does not imply offline input.

| Entry point | Result | Default selection | Error behavior |
| --- | --- | --- | --- |
| `raw.scan_entries()` | `RawMftEntry` | Records `24..`, excludes bitmap-unused slots, skips extensions | Invalid records skipped; scan I/O errors returned |
| `raw.parallel().rich()` | Batches of `RawMftEntry` | Same raw defaults | Same tolerant parsing; scanner/callback errors end the operation |
| `raw.parallel().lean()` | Batches of `RawMftLeanEntry` | Same raw defaults | Same tolerant parsing; scanner/callback errors end the operation |
| `stream.iter_records()` | `MftRecord` per valid record position | All record positions from zero, including unused and extensions | Parse errors returned per position and iteration continues; I/O error stops iteration |

MftStream extension I/O failures are attached to the base record's diagnostics;
the iterator yields that partial record, then a terminal `MftError::ExtensionRead`,
then stops. Standard `Result` collection therefore reports incomplete scans as
errors. Ordinary live volume scans use the bitmap for selection;
`bitmap_usage(number)` returns `None` for unavailable
evidence or out-of-range slots. Unknown allocation is scanned conservatively;
inspect each entry's `is_used`. Serial scan I/O errors are terminal.
A successful raw scan does not prove complete metadata. Live volumes are not
atomic snapshots.

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

There is deliberately no terminal scan operation on `raw.parallel()` until the
caller chooses `.rich()` or `.lean()`. For application-specific callback errors,
use the `try_*` terminal methods and match `ScanError::Scanner` or
`ScanError::Consumer`. Worker callbacks already in progress may finish after
another callback fails; completed side effects are not rolled back.

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
