# Logical MFT streams

`MftStream<R: Read + Seek>` parses every logical MFT slot and retains record
errors and metadata evidence. It owns a reader or borrows one such as `&mut R`.
Use `from_path` for a read-only buffered file, `from_reader` for a file/buffer,
or `from_raw_mft` for a volume. Offline files need ordinary read permissions;
live sources retain [platform access requirements](../README.md#limits-and-access-requirements).

## Input and addressing

- Input must be the logical contents of an extracted MFT, with record zero at
  offset zero and all slots retained. Whole disk images, carved/concatenated
  records, and compressed input are unsupported.
- Supply original USA-protected bytes. Fixups use a private buffer and never
  change the source; already fixed-up records are unsupported.
- `MftStreamOptions::new(size)` requires a power-of-two record size from 512
  through 65,536 bytes. It does not guess geometry. Each FILE header must agree
  with that size and its USA count.
- Construction seeks to the end to establish a fixed slot count, then to zero.
  Empty input is valid; a partial trailing slot returns `TruncatedRecord`.
  Keep the source unchanged while the parser owns or borrows it.
- `read_record(number)` addresses `number * record_size`, independent of
  optional header fields. `stream_offset` is an offset in the logical MFT,
  never in the volume.

The parser reuses one base-record buffer; extension reads use separate buffers.
Returned records own their metadata. The iterator exclusively borrows the stream.
Random and extension reads do not change iteration order; `into_inner()` makes
no promise about the final cursor position.

## Errors and iteration

`iter_records()` starts at zero and includes system, unused, and extension
records. Non-FILE or malformed slots produce item-level errors and iteration
continues. Base-record I/O errors are reported once and terminate iteration.

Extension I/O failures yield the partial base record with a diagnostic, then
`MftError::ExtensionRead` once before stopping. That error identifies a dependency,
not another slot, and shares its original source with the diagnostic.
A direct `read_record` call returns the partial record with diagnostics without
the iterator's extra terminal error. Explicit reads or restarting iteration
remain allowed.

`collect::<MftResult<Vec<_>>>()` therefore reports terminal I/O failures, but
successful collection still requires checking metadata and diagnostics for
recoverable limitations.

## Metadata and payloads

Interpret zero, empty, and `None` fields in `MftRecord.entry` alongside
`metadata`, `diagnostics`, and `external_payloads`:

| Metadata state | Meaning |
| --- | --- |
| `Present { complete: true }` | Supported metadata was fully inspected |
| `Present { complete: false }` | Some metadata was observed; the category is incomplete |
| `Absent` | No attribute was found in the fully inspected record set |
| `Unknown` | Inspection could not establish presence or absence |

Categories cover standard information, names, unnamed/named data, and reparse
tags. Presence is attribute-based: empty resident DATA is present, and zero
timestamps do not establish absence. An incomplete attribute list can affect
all categories. Physical extension records remain incomplete because their
base was not inspected.

Completeness concerns supported metadata, not every NTFS attribute or file
contents. Nonresident headers can provide sizes without their payload.
`external_payloads` identifies observed nonresident attributes by record number,
type, and instance ID; an empty list does not prove absence after incomplete
inspection.

There is no general file-content API or external-payload callback.
Nonresident attribute lists and reparse payloads produce
`ExternalPayloadUnavailable` diagnostics. Runlist LCNs are never offsets into
the extracted MFT. Preserve evidence when exporting; serialization and paths
belong to consumers.

## Extension records

Extension resolution is disabled by default. Base records with an attribute
list report `ExtensionsNotResolved`; this physical-record mode is suitable
for differential comparisons.

With `resolve_extensions(true)`, resident lists select attributes by type,
instance ID, raw UTF-16 name, and lowest VCN. Sequence numbers and full base
references must match. Repeated references are deduplicated and failed reads
cached for that base lookup; unrelated attributes are not merged.

Missing records, mismatched references, malformed attributes/lists, and nested
lists produce diagnostics while retaining usable metadata. Resolution follows
one level, including checking self-references against the base rather than
recursing. Nonresident lists remain unresolved even with volume input.

For split data streams, the first segment supplies sizes; continuation segments
do not overwrite them. Runlists are not aggregated, and `SplitDataStream` marks
data completeness false.

## Volume-backed input

`RawMft::byte_reader()` maps fragmented `$MFT::$DATA` into a logical `Read + Seek`
source. For example, to enable extension inspection without extracting a file:

```rust,no_run
use ntfs_mft::{MftResult, MftStream, MftStreamOptions, Volume};

fn inspect(volume: &Volume) -> MftResult<()> {
    let raw = volume.raw_mft()?;
    let options = MftStreamOptions::new(raw.file_record_size())?
        .resolve_extensions(true);
    let mut parser = MftStream::from_reader(raw.byte_reader()?, options)?;
    for result in parser.iter_records().take(100) {
        println!("{}", result?.entry);
    }
    Ok(())
}
```

The adapter reopens the volume read-only and can outlive the original volume
and `RawMft`. Reopening can fail; keep the mounted source stable during construction.

Reads preserve original USA bytes, including records split across extents.
Sparse ranges and the uninitialized tail read as zero. EOF and `SeekFrom::End`
use logical DATA length; `RawMft::record_count()` uses extent capacity.
Seeks beyond EOF are allowed and then read zero bytes. Negative or overflowing
seeks fail without changing the cursor. Reads may stop at an extent boundary;
`read_exact` handles these short reads.

Compressed/encrypted MFT streams and inconsistent size/mapping bounds are
rejected. Geometry is fixed at construction; live input is not a snapshot and
does not follow MFT growth. Only MFT DATA is exposed: backing the parser with
a volume does not make external attribute payloads available.

## Coverage

[`tests/offline_mft.rs`](../tests/offline_mft.rs) covers public stream parsing,
geometry, USA corruption, truncation, extension selection, partial metadata,
and I/O failures. Byte-reader tests cover fragmented and split records, sparse
and uninitialized ranges, EOF, offsets, short reads, and overflow.

The [differential harness](../differential-tests/README.md) compares fixture
records and backwards reads through a simulated fragmented device. See
[contributor guidance](../CONTRIBUTING.md#live-device-tests) for separate
live-device checks.
