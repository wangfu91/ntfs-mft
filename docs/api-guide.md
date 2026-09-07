# Choosing an MFT API

Use a raw scan for a best-effort base-record listing from a volume. Use a
diagnostic raw scan when skipped-record counts and metadata evidence matter.
Use `MftStream` when each physical slot, including invalid or extension slots,
needs an explicit outcome.

| Entry point | Result | Default selection | Error behavior |
| --- | --- | --- | --- |
| `raw.try_iter()` | `RawMftEntry` | Records `24..`, excludes bitmap-unused slots, skips extensions | Invalid records skipped; scan I/O errors returned |
| `raw.parallel().rich()` | Batches of `RawMftEntry` | Same raw defaults | Same tolerant parsing; scanner/callback errors end the operation |
| `raw.parallel().lean()` | Batches of `RawMftBatchEntry` | Same raw defaults | Same tolerant parsing; scanner/callback errors end the operation |
| `raw.try_iter_diagnostic(options, limit)` | `RawMftDiagnosticRecord` plus final report | Supplied raw selection | Counts skipped categories, retains bounded parser errors; scan I/O error returned once, then stops |
| `stream.iter_records()` | `OfflineMftRecord` per valid slot | All physical slots from zero, including unused and extensions | Parse errors returned per slot and iteration continues; I/O error stops iteration |

Offline extension I/O failures are attached to the base record's diagnostics;
the iterator yields that record and then stops. Ordinary raw scans use the
bitmap for selection; if it is unavailable, inspect each entry's `is_used`.
Neither a successful raw scan nor reaching the end of a diagnostic scan proves
complete metadata. Live volumes are not atomic snapshots.

## Selection and metadata are separate choices

```rust
use ntfs_mft::{RawMftEntryOptions, RawMftScanOptions};

let metadata = RawMftEntryOptions::builder()
    .collect_alternate_data_streams(false)
    .collect_data_run_summary(false)
    .build();
let options = RawMftScanOptions::builder()
    .range(0..1000)
    .include_unused_records(true)
    .entry(metadata)
    .build();
```

Raw scan options default to `24..`; standalone `RawMftRecordRange::default()`
represents `0..`. All standard `u64` Rust ranges are accepted by `.range()`.
The same rich options govern serial scanning and both planning and execution
of rich parallel scans. `RawMftChunkPlanOptions` controls partition size only.
Explicit chunks are clipped to selection and volume bounds; overlapping chunks
are rejected. Results are delivered in supplied chunk order.

Rich entries include timestamps, attributes, ADS, and data-run summaries,
subject to `collection_options`. Lean entries keep names, references, sizes,
links, and basic flags; their options expose no ADS or run-summary toggles.
`options.into_lean()` explicitly retains selection/buffers/DOS-link policy and
discards the rich-only metadata choices.

```rust,no_run
use ntfs_mft::{MftResult, RawMft, RawMftScanOptions};

fn visit(raw: &RawMft<'_>) -> Result<(), ntfs_mft::ScanError<std::convert::Infallible>> {
    let options = RawMftScanOptions::builder().range(24..).build();
    raw.parallel().rich().scan_options(options).try_for_each_batch(|batch| {
        for entry in batch.entries {
            println!("{}: {}", entry.file_reference, entry.file_name);
        }
        Ok(())
    })
}
```

There is deliberately no terminal scan operation on `raw.parallel()` until the
caller chooses `.rich()` or `.lean()`. For application-specific callback errors,
use the `try_*` terminal methods and match `ScanError::Scanner` or
`ScanError::Consumer`. Worker callbacks already in progress may finish after
another callback fails; completed side effects are not rolled back.

## Preserve evidence, names, and identity

Use `RawMftDiagnosticRecord.metadata` to distinguish absent, present, unknown,
and not-collected fields. Keep its diagnostics and the iterator's final report.
Raw attribute-list enrichment remains selective and is reported as unverified;
use the offline inspection API for its supported extension-validation evidence.
`MftStreamOptions::new(size)` defaults to no extension resolution; enable
`.resolve_extensions(true)` for one level of resident attribute-list references.
An MFT-only source cannot recover arbitrary nonresident attribute payloads.

`NtfsName` retains exact UTF-16 for selected names, links, and ADS on every host.
Use `as_utf16()` for identity/export, `try_to_string()` for checked Unicode,
and `to_string_lossy()` or `Display` for readable text. `to_os_string()` is
lossless on Windows; Unix native paths replace unpaired surrogates.

`NtfsFileReference::try_from_parts(record, sequence)` rejects record-number
overflow. Its record and sequence accessors return values directly. Convert
explicitly to generic `Fid` when a Windows API needs it. A record number alone
is not durable identity because deleted slots can be reused.

Build `MftPathIndex` during an existing scan to avoid an extra scan. Include root
record 5 and all required parents. Strict resolution verifies full references;
best-effort resolution reports fallback or partial suffixes. Native path quality
describes reference matching, not UTF-16 conversion fidelity. The convenience
`raw.path_resolver()` performs an additional full scan and stores an in-memory
index; it cannot guarantee coverage when malformed records were skipped.

## Parsing a logical stream

`MftStream::from_path(path, options)` and `from_reader(reader, options)` require
an explicit `usize` record size. If you already have a `RawMft`, use
`MftStream::from_raw_mft(&raw)` to infer geometry and open an independent handle.
The resulting stream can outlive `raw` and its volume. For custom options, use
`from_reader(raw.byte_reader()?, options)` with `raw.file_record_size()`.

See [raw scan details](raw-mft-read.md), [offline inspection](offline-mft.md),
and [versioned export](export.md) for their full contracts.
