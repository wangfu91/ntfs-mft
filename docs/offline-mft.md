# Logical MFT stream parsing

`MftStream` parses a logical `$MFT` stream. An **offline MFT scan** uses an
already extracted file or buffer as that stream; `from_raw_mft()` and
`byte_reader()` instead derive it from a live volume, retaining that source's
consistency limitations.
Single-record and byte operations are called **reads**.

`MftStream<R: Read + Seek>` owns a reader (or a borrowed reader such as `&mut R`).
`from_path` uses a read-only buffered file. `Cursor<Vec<u8>>` and `Cursor<&[u8]>`
support owned and borrowed buffers. No device access or elevated privileges are required.

## Reading directly from a volume

`RawMft::byte_reader()` supplies a `MftByteReader` implementing `Read + Seek`.
It maps the volume's fragmented `$MFT::$DATA` to logical byte offsets, so the
same parser works without an intermediate extracted file:

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

On Linux and macOS, open `Volume` using a device path or mount point instead. The adapter
reopens that volume source read-only and owns an independent handle, so it can
outlive the original `RawMft` and `Volume`. Construction can fail if the source
cannot be reopened. Do not change the mounted source during construction.

Reads preserve original USA bytes, including when a record crosses extents.
Sparse ranges and the uninitialized tail read as zero. EOF and `SeekFrom::End`
use the `$DATA` logical length, not allocated extent capacity; `RawMft`'s existing
capacity-based `record_count()` is unchanged. Seeks beyond EOF are allowed and
subsequent reads return zero bytes. Negative/overflowing seeks fail without
changing the logical cursor. Reads may stop at an extent boundary; `read_exact`
and normal buffered consumers handle these standard short reads.

Compressed/encrypted `$MFT` streams and inconsistent size/mapping bounds are
rejected. Geometry is captured at construction. A live scan is not a filesystem
snapshot and does not follow subsequent MFT growth; use a stable source for
consistent results. The adapter exposes only `$MFT::$DATA`: external payloads
remain unavailable to `MftStream`, even though the backing source is a volume.

## Input and addressing

- Supply the logical contents of an extracted `$MFT`, retaining all record slots.
  Offset zero must correspond to record zero. Disk images, carved or concatenated records, and compressed input are unsupported.
- Supply original USA-protected bytes. Fixup happens on a private record buffer;
  the source is never changed. Already fixed-up records are unsupported.
- `MftStreamOptions::new(size)` accepts powers of two from 512 through 65536.
  Record size is required; the reader never guesses from the first header.
  Each FILE record must agree with the configured allocated size and USA count.
- Construction seeks to the end to establish a fixed slot count, then to zero.
  An empty stream is valid. A partial trailing slot is `TruncatedRecord`.
  The source must remain unchanged until `into_inner` returns it.
- `read_record(number)` takes `&mut self`. It addresses a logical record position at
  `number * record_size`. Slot numbering does not come from optional header fields.
  `MftRecord::stream_offset` always refers to this stream, never the volume.
- `iter_records()` starts at zero and yields every slot, including system records,
  unused records and extension records. A non-FILE slot is an item-level error.
  Parse errors allow iteration to continue. I/O errors terminate iteration after
  one report. Extension I/O errors are attached to the yielded base entry and also
  terminate iteration. Restarting iteration or explicitly reading a slot is allowed.
- The iterator exclusively borrows the stream. Extension reads use the same
  reader; subsequent slot reads seek explicitly, so random lookups cannot disturb
  iteration order. `into_inner` makes no promise about the final cursor position.

## Metadata and unavailable payloads

`MftRecord.entry` reuses `RawMftEntry`. Its zero, empty and `None` defaults
must be interpreted alongside `metadata`:

| State | Meaning |
| --- | --- |
| `Present { complete: true }` | Supported metadata was observed and inspected completely. |
| `Present { complete: false }` | Some metadata was observed; the category is incomplete. |
| `Absent` | No attribute was found in the fully inspected record set. |
| `Unknown` | Nothing valid was observed, but inspection could not establish absence. |

The categories are standard information, file names, unnamed data, named data,
and reparse tags from `$REPARSE_POINT`. Presence is attribute-based: an empty
resident `$DATA` is present, and zero timestamps do not establish absence.
An incomplete list can affect all categories. A physical extension record's
categories remain incomplete because its base was not inspected.

Completeness covers the supported metadata representation, not every NTFS
attribute type and not file-content availability. Unknown attributes are not
decoded into `RawMftEntry`. Non-resident data headers can provide valid sizes and
valid sizes while their contents remain outside the input. `external_payloads`
identifies observed non-resident attributes by record number, type and instance
ID. An empty list is not evidence of absence if inspection was incomplete.

The API exposes no general content-reading method, even for resident data.
There is no volume-payload callback in this version. Non-resident attribute lists
and reparse payloads produce `ExternalPayloadUnavailable` diagnostics. Runlist
LCNs are never interpreted as offsets into the extracted MFT.

## Extension records

Default mode builds metadata from each MFT record only. A base record with
an attribute list reports `ExtensionsNotResolved`. Use this mode for physical-record comparisons.

With `resolve_extensions(true)`, resident lists select attributes using type,
instance ID, raw UTF-16 name and lowest VCN. Extension record sequence numbers
and full base references must match before attributes are accepted. Repeated
references are deduplicated and failed reads are cached for that base lookup.
Unrelated extension attributes are not merged. All accepted attributes use the
existing entry builder's name selection and metadata construction logic.

Missing records, mismatched references, malformed attributes/lists and nested
lists produce structured diagnostics, retaining usable base metadata. Nested
lists are not followed, including cycles back to the base. A self-reference in
the base list is checked against the base's own attribute, not read recursively.
Non-resident lists cannot be resolved without the original volume.

Split data streams are only partially represented: the first segment supplies
sizes, continuation segments do not overwrite them, and `SplitDataStream`
marks data completeness false. Continuation runlists are not aggregated.

## Tests and downstream integration

`cargo test --test offline_mft` exercises the public API with synthetic records
and the pinned LFS fixture. Synthetic cases cover corrupted USA bytes, geometry,
truncation, missing/mismatched/recursive extensions, precise attribute selection,
empty vs absent data, unavailable payloads, and injected I/O failures. Ensure the
fixture is hydrated before building; its expected checksum is documented beside it.

Differential tests can compare `.entry` in default physical-record mode against
their independent parser. They should compare per-slot errors separately, and
account for explicit rejection of malformed payloads.
Export consumers should preserve the metadata states and diagnostics, rather than
converting unknown values to zero, empty strings or null without explanation.
See [JSONL export](export.md) for optional serialization. MFT-stream parsing
does not reconstruct paths or provide parallel scans.
