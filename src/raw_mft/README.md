# Raw MFT implementation

| Module | Responsibility |
| --- | --- |
| `layout/` | NTFS structures, runlists, extent mapping, and USA fixups |
| `bootstrap/` | Boot geometry, MFT stream discovery, and allocation bitmap |
| `volume_reader.rs`, `reader.rs` | Aligned volume reads and record access |
| `entry_build/`, `attr_list.rs` | Metadata construction and extension enrichment |
| `serial/`, `parallel/` | Record scanning and worker execution |
| `chunk_plan.rs`, `options.rs` | Logical chunk ranges and scan configuration |

The read path is boot sector → MFT runlist → extent map → FILE record → USA
fixup → attributes. Extent lookup handles fragmented storage; each parallel
worker owns its readers and shares immutable discovery data.

See [reader internals](../../docs/raw-mft-read.md) for behavior and constraints,
[serial findings](../../docs/raw-mft-serial-read-findings.md) and
[parallel findings](../../docs/raw-mft-parallel-ingest-findings.md) for measured
performance. Use Criterion benchmarks for timing and the matching profile
examples for flamegraphs or ETW.
