# Offline MFT parsing

`MftStream<R: Read + Seek>` owns a reader (or a borrowed reader such as `&mut R`).
`from_path` uses a read-only buffered file. `Cursor<Vec<u8>>` and `Cursor<&[u8]>`
support owned and borrowed buffers. No volume discovery, boot sector, device
handle, cluster size, or Administrator privileges are required.

## Input and addressing

- Supply the logical contents of an extracted `$MFT`, retaining all record slots.
  Offset zero must correspond to record zero. This is not a disk-image parser,
  carving interface, arbitrary record concatenation format, or decompressor.
- Supply original USA-protected bytes. Fixup happens on a private record buffer;
  the source is never changed. Already fixed-up records are unsupported.
- `MftStreamOptions::new(size)` accepts powers of two from 512 through 65536.
  Record size is required; the reader never guesses from the first header.
  Each FILE record must agree with the configured allocated size and USA count.
- Construction seeks to the end to establish a fixed slot count, then to zero.
  An empty stream is valid. A partial trailing slot is `TruncatedRecord`.
  The source must remain unchanged until `into_inner` returns it.
- `read_record(number)` takes `&mut self`. It addresses a physical slot at
  `number * record_size`. Slot numbering does not come from optional header fields.
  `OfflineMftRecord::stream_offset` always refers to this stream, never the volume.
- `iter_records()` starts at zero and yields every slot, including system records,
  unused records and extension records. A non-FILE slot is an item-level error.
  Parse errors allow iteration to continue. I/O errors terminate iteration after
  one report. Extension I/O errors are attached to the yielded base entry and also
  terminate iteration. Restarting iteration or explicitly reading a slot is allowed.
- The iterator exclusively borrows the stream. Extension reads use the same
  reader; subsequent slot reads seek explicitly, so random lookups cannot disturb
  iteration order. `into_inner` makes no promise about the final cursor position.

## Metadata and unavailable payloads

`OfflineMftRecord.entry` reuses `RawMftEntry`. Its zero, empty and `None` defaults
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
run summaries while their contents remain outside the input. `external_payloads`
identifies observed non-resident attributes by record number, type and instance
ID. An empty list is not evidence of absence if inspection was incomplete.

The API exposes no general content-reading method, even for resident data.
There is no volume-payload callback in this version. Non-resident attribute lists
and reparse payloads produce `ExternalPayloadUnavailable` diagnostics. Runlist
LCNs are never interpreted as offsets into the extracted MFT.

## Extension records

Default mode builds metadata from each physical record only. A base record with
an attribute list reports `ExtensionsNotResolved`. This mode is useful for
record-by-record differential comparisons.

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
sizes and its run summary, continuation segments do not overwrite them, and
`SplitDataStream` marks data completeness false. This version does not aggregate
all continuation runlists. Uncertainty is intentionally conservative.

## Tests and downstream integration

`cargo test --test offline_mft` exercises the public API with synthetic records
and the pinned LFS fixture. Synthetic cases cover corrupted USA bytes, geometry,
truncation, missing/mismatched/recursive extensions, precise attribute selection,
empty vs absent data, unavailable payloads, and injected I/O failures. Ensure the
fixture is hydrated before building; its expected checksum is documented beside it.

Differential tests can compare `.entry` in default physical-record mode against
their independent parser. They should compare per-slot errors separately, and
account for explicit rejection of malformed payloads. The existing private
fixture parser and raw-volume APIs retain their behavior.

Export consumers should preserve the metadata states and diagnostics, rather than
converting unknown values to zero, empty strings or null without explanation.
There are no serialization dependencies, output formats, path reconstruction,
parallel offline scans, or assumed sibling-task interfaces in this change.
