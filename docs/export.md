# Version-one scan export

Enable `serde` for `ntfs_mft::export` DTOs (`Serialize` and `Deserialize`). Enable
`jsonl` for those DTOs plus `JsonlWriter<W: Write>`. Neither feature is enabled by
default. Serde, JSON serialization, and UTC formatting dependencies are optional;
schema generation and validation dependencies are development-only.

```toml
ntfs-mft = { version = "0.1", features = ["jsonl"] }
```

```rust,no_run
use ntfs_mft::{MftResult, MftStream, MftStreamOptions};
use ntfs_mft::export::JsonlWriter;
use std::io::{self, BufWriter};

fn main() -> MftResult<()> {
    let mut source = MftStream::from_path("extracted.MFT",
        MftStreamOptions::new(1024)?.resolve_extensions(true))?;
    let mut output = JsonlWriter::new(BufWriter::new(io::stdout().lock()));
    for result in source.iter_records() {
        match result {
            Ok(record) => output.write_offline(&record)?,
            Err(error) => output.write_error(&error)?,
        }
    }
    output.flush()
}
```

The example supports both merged offline parsing and existing volume scans:

```text
cargo run --example raw_mft_export --features jsonl -- file extracted.MFT 1024
cargo run --example raw_mft_export --features jsonl -- volume C
cargo run --example raw_mft_export --features jsonl -- volume /dev/nvme0n1p3
```

Redirect stdout to save JSONL. Progress and top-level failures go to stderr. Input
files open read-only. Only volume mode needs device permissions (Administrator
on Windows). Offline mode resolves one level of resident extension lists. Volume
mode starts at record zero, includes unused records, and collects ADS, run summaries,
and DOS aliases. The scanner may skip non-FILE slots. Output contains only items
the iterator yields, not an inventory of every physical slot. The example returns
nonzero for top-level or item errors; partial metadata is a record with diagnostics,
not an item error. Consult those diagnostics even when the process succeeds.

## Wire contract

[export-v1.schema.json](export-v1.schema.json) defines each complete line. The
envelope has `schema_version: "1"`, `type: "record" | "error"`, and `data`.
Each line is compact UTF-8 JSON followed by LF; embedded control characters are
escaped. No header, footer, BOM, or whole-scan array is emitted. An empty iterator
produces an empty file. Unknown schema versions fail DTO deserialization.

The public DTOs are independent of scanner structs. Internal fields are mapped
explicitly, so adding a scanner field cannot silently add a wire field. Schema
and golden checks gate representation changes. Breaking changes require a new
version. Readers should tolerate additional object fields within v1. DTO roundtrip
means preserving exported data, not reconstructing the original disk record or
validating caller-authored DTOs against NTFS invariants.

- All u64/u128 identifiers, sizes, offsets, cluster totals, and FILETIME values
  are canonical unsigned decimal strings. Small flags, namespaces, counts, and
  u16 sequences are JSON integers. Unknown flag bits survive.
- A file reference includes its full value and width. NTFS also carries its lower
  48-bit record number and upper 16-bit sequence. Extended caller-supplied IDs
  have null NTFS components. Header record/sequence fields remain separately
  represented; conversion does not repair inconsistent caller-created entries.
- `base_record_number` preserves the scanner's value (zero sentinel included).
  `base_sequence_number` is null because both current adapters receive entries
  whose base sequence bits were discarded. Never interpret null as sequence zero.
- Timestamps retain raw unsigned ticks since 1601. UTC text uses seven fractional
  digits and Z. Dates beyond four-digit years have null text and `out_of_range`.
  Zero retains its epoch text with `zero_ambiguous`; metadata determines whether
  its attribute was absent, unknown, or actually observed. No local timezone or
  platform `SystemTime` conversion is involved.
- Names and supplied paths carry `display` plus `raw: { encoding, units }`.
  `windows_utf16` preserves u16 units, including unpaired surrogates. `unix_bytes`
  preserves u8 OS bytes. These arrays can be decoded on either platform; native
  path reconstruction is left to the caller. Display text is never authoritative.
  `name_fidelity` explicitly distinguishes Windows preservation from Linux's
  unavailable original UTF-16: the Linux parser may have replaced lone surrogates
  before export. Export cannot undo that loss. Namespace codes likewise represent
  the scanner's normalized enum, not unknown original bytes.
- `selected_name` is separate from retained `links`; each link keeps its parent
  sequence and namespace. Preserve array order. Header hard-link count can differ
  from array length because of DOS aliases, collection options, or incomplete
  inspection. FILE_NAME timestamps belong only to the selected name.
- `unnamed_stream` is null when no unnamed data was observed. A present empty
  stream retains zero sizes. Named ADS remain separate array entries, including
  duplicate names/segments if supplied. ADS flags and per-link timestamps not
  supplied by the scanner are not invented. Run summaries cover represented
  segments, not necessarily a whole split stream. No file contents are exported.

## Evidence, errors, and paths

`from_offline` preserves category-level `present`, `partial`, `absent`, and
`unknown` states, stream offsets, external-payload headers, and structured
diagnostics. The offline completeness definition covers supported metadata, not
original name-byte fidelity or arbitrary file contents. Diagnostic codes are:
`malformed_attribute`, `extensions_not_resolved`, `external_payload_unavailable`,
`malformed_attribute_list`, `extension_read`, `extension_mismatch`,
`nested_attribute_list`, `split_data_stream`, and `extension_record_only`.
Extension-read issues retain the full requested reference and nested error.

`from_entry(&entry, &ExportContext)` and `write_entry` support existing serial,
parallel, or caller-owned results. Supply the actual `RawMftEntryOptions` when
known. `not_collected` means disabled, `not_observed` does not establish absence,
and `partial` indicates an observed value without completeness evidence. SI and
reparse-attribute evidence remain unknown. No timestamp-zero or empty-vector
heuristic establishes absence. Empty raw diagnostics do not establish success of
enrichment, since raw scans do not retain those failures.

Error categories are `io`, `parse`, and `other`; message text is diagnostic only.
Record numbers and volume offsets are supplied only when the original error has
them. `reported_offset` retains offsets whose address space the generic error
does not identify; it is not relabeled a volume or extracted-stream offset.

Paths default to `not_requested`. Callers may set `ExportRecordV1.path` to
`resolved`, `unresolved`, or `partial`, then use `write_record`. The path describes
the selected name, not every hard link. Export does not allocate a directory tree,
query live paths, or substitute a leaf name when resolution fails.

## Streaming and failure behavior

The writer retains one serialized item's buffer and conversion DTO, never the
entire scan. Memory is proportional to the largest item, plus caller/scanner
buffers. Parallel callers must serialize access to the writer; output follows
call order and is not sorted. The synchronous API naturally applies backpressure.

Serialization completes in the item buffer before sink output begins. Sink
short writes are handled with `write_all`. A sink failure may truncate the last
line; earlier complete lines remain usable. A write or flush failure poisons the
writer, preventing further appends. Output is not transactional. Call `flush` or
`finish` and check the result; dropping the writer is not a successful flush.
Use `?` on each write so a failed sink stops scan consumption immediately.

CSV is intentionally deferred: one row per record cannot faithfully represent
arbitrary links, streams, and diagnostics. A follow-up should define separate
record/link/stream tables or explicitly document its lossy flattening policy.

## Validation

`cargo test --features jsonl export::` checks the schema snapshot, golden error
line, fixture-backed DTOs, Unicode edge cases, exact numbers and dates, metadata
states, short writes, output failure, flush failure, and iterator backpressure.
`serde`-only builds exercise DTOs without the JSONL writer. Schema tests use
Schemars only on DTOs and validate actual output with JSON Schema. To intentionally
regenerate the reviewed contract, set `NTFS_MFT_UPDATE_EXPORT_SCHEMA=1` while
running `schema_and_golden_contract`, then inspect the schema/golden diff.
