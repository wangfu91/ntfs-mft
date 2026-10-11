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

Start with the [API guide](../../docs/api-guide.md) for public entry points and
the [stream contract](../../docs/offline-mft.md) for per-slot parsing.
[Reader internals](../../docs/raw-mft-read.md) describe the scan pipeline;
[contributor guidance](../../CONTRIBUTING.md#benchmarks-and-profiling) covers
benchmarks and matching profile targets.
