# Raw MFT implementation

| Module | Responsibility |
| --- | --- |
| `layout/` | NTFS structures, runlists, extent mapping, and USA fixups |
| `bootstrap/` | Boot geometry, MFT stream discovery, and allocation bitmap |
| `volume_reader.rs`, `reader.rs` | Aligned volume reads and record access |
| `entry_build/`, `attr_list.rs` | Metadata construction and extension enrichment |
| `stream.rs`, `inspection.rs`, `metadata.rs` | Logical stream parsing, attribute validation, and metadata evidence |
| `byte_reader.rs` | Owned `Read + Seek` view of the volume's logical MFT bytes |
| `serial/`, `parallel/` | Record scanning and worker execution |
| `chunk_plan.rs`, `options.rs` | Logical chunk ranges and scan configuration |

The read path is boot sector → MFT runlist → extent map → FILE record → USA
fixup → attributes. Extent lookup handles fragmented storage; each parallel
worker owns its readers and shares immutable discovery data.

Start with the [API guide](../../docs/api-guide.md) for public entry points and
the [stream contract](../../docs/offline-mft.md) for per-slot parsing. See
[reader internals](../../docs/raw-mft-read.md) for volume behavior and constraints,
[serial findings](../../docs/raw-mft-serial-read-findings.md) and
[parallel findings](../../docs/raw-mft-parallel-ingest-findings.md) for measured
performance. Use Criterion benchmarks for timing and the matching profile
examples for flamegraphs or ETW.
