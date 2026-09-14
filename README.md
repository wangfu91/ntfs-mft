# ntfs-mft

Read-only NTFS Master File Table parsing and scanning for Windows, Linux, and macOS.

This library reads the boot sector and fragmented `$MFT` streams directly,
validates FILE records and update sequence arrays, and exposes timestamps,
names, sizes, hard links, alternate data streams, and file attributes. It includes
serial and parallel live volume scans, indexed path resolution, and experimental historical
path reconstruction.

See [Choosing an MFT API](docs/api-guide.md) for scan defaults, metadata choices,
error behavior, and migration examples.

## Terminology

- **Live volume scan** reads MFT records directly from an NTFS volume/device.
- **Offline MFT scan** parses an already extracted logical `$MFT` file or buffer.
- **Read** means a single-record read or byte-stream operation; **scan** means
  traversing records. **Parse** means interpreting the NTFS record bytes.

`RawMft` accesses a volume and supports live volume scans. `MftStream` parses a
logical byte stream from either source: a file-backed stream is an offline MFT
scan; `MftStream::from_raw_mft()` still reads a live volume. Source and parsing
policy are separate. A live volume scan is not an atomic filesystem snapshot.

## Installation

Requires Rust 1.88 or newer. For the crates.io release:

```toml
[dependencies]
ntfs-mft = "0.1"
```

Version 0.1.0 is being prepared for its first publication. Until it is available
on crates.io, clone this repository and use a local path dependency:

```toml
[dependencies]
ntfs-mft = { path = "../ntfs-mft" }
```

```rust,no_run
use ntfs_mft::{MftResult, Volume};

fn main() -> MftResult<()> {
    #[cfg(windows)]
    let volume = Volume::from_drive_letter('C')?;
    #[cfg(target_os = "linux")]
    let volume = Volume::from_mount_point("/media/user/windows")?;
    #[cfg(target_os = "macos")]
    let volume = Volume::from_mount_point("/Volumes/ESD-ISO")?;
    let mft = volume.raw_mft()?;
    for entry in mft.try_iter()?.take(20) {
        println!("{}", entry?.file_name.to_string_lossy());
    }
    Ok(())
}
```

## Offline MFT scans

`MftStream` reads a standalone logical `$MFT` file using ordinary file permissions.
It also accepts `Read + Seek` sources through `from_reader`, including
`Cursor<&[u8]>` for borrowed memory:

```rust,no_run
use ntfs_mft::{MftResult, MftStream, MftStreamOptions};

fn main() -> MftResult<()> {
    let options = MftStreamOptions::new(1024)?;
    let mut mft = MftStream::from_path("extracted.MFT", options)?;
    for record in mft.iter_records() {
        match record {
            Ok(record) => println!("{} {:?}", record.entry, record.metadata),
            Err(error) => eprintln!("{error}"),
        }
    }
    Ok(())
}
```

Record size is explicit; there is no automatic geometry detection. Iteration
includes system, unused, extension, and non-FILE slots. Each bad slot returns an
error. Enable `options.resolve_extensions(true)` for one level of resident
attribute-list references. Always retain `metadata`, `diagnostics`, and
`external_payloads` when exporting: a missing value may be unknown, not absent.
An MFT-only stream cannot supply non-resident attribute payloads or arbitrary
file contents. See [the offline API contract](docs/offline-mft.md).

For direct volume input, use `MftStream::from_raw_mft(&raw)` with an existing
`RawMft`. It discovers the record size and opens an independent handle. For custom
parser options, pass `raw.byte_reader()?` to `MftStream::from_reader` and use
`raw.file_record_size()`. The adapter handles fragmented extents through
`Read + Seek`. Raw-volume permissions and live-scan consistency limits still apply.

## Scope and limitations

- NTFS only; ReFS is rejected. For USN journal APIs check [usn-journal-rs](https://crates.io/crates/usn-journal-rs).
  Windows FSCTL enumeration is used only by local parity-test helpers.
- Windows raw volume access requires Administrator privileges and a launch context
  permitting raw-volume I/O; see [MSIX runtime requirements](docs/windows-raw-volume-access.md).
  Linux and macOS require read permission for the backing device (typically root
  on macOS); see [macOS access and validation](docs/macos-raw-volume-access.md).
  All device opens are read-only.
- A live scan is not an atomic filesystem snapshot; concurrent changes can affect results.
- Attribute-list enrichment is one level deep. Historical paths are best effort.
- Snapshot paths are the default; Windows live fallback is explicitly opt-in.

## Optional JSONL export

Enable `jsonl` for streaming export of raw scan results or offline `MftStream`
records. Enable only `serde` for the separate versioned DTOs. Both are optional.
The [export guide and schema](docs/export.md) describe exact identifiers,
timestamps, lossless NTFS names, native path encodings, metadata completeness, and known parser limits.

```text
cargo run --example raw_mft_export --features jsonl -- file extracted.MFT 1024
```

## Development

Install [Git LFS](https://git-lfs.com/) before cloning, then run `git lfs pull`
from the repository root. Tests and packaging need the actual fixture bytes;
Git LFS pointer files are not usable fixtures.

```text
cargo build
cargo fmt --all -- --check
cargo test
cargo test --manifest-path differential-tests/Cargo.toml --locked --lib differential_tests
cargo clippy --all-targets --all-features -- -D warnings
cargo package
cargo run --example raw_mft_serial_read -- C
cargo run --example raw_mft_parallel_chunks -- C
cargo run --example deletion_forensic -- C
cargo bench --bench raw_mft
cargo bench --bench raw_mft_ingest
cargo bench --features windows-path-resolver-bench --bench path_resolver
```

On Linux and macOS, pass an NTFS mount point or `/dev/...` device instead of `C`.
Device tests skip when privileges or a suitable volume are unavailable.
Tests and benchmarks use `NTFS_MFT_TEST_DRIVE` on Windows and
`NTFS_MFT_TEST_VOLUME` on Linux/macOS. ReFS rejection tests use
`NTFS_MFT_REFS_TEST_DRIVE`. Benchmark tuning uses `NTFS_MFT_BENCH_*` and
`NTFS_MFT_SERIAL_*`.

See [Windows API parity tests](integration-tests/README.md),
[offline parser differential tests](differential-tests/README.md), and
[implementation](docs/raw-mft-read.md).

See [CONTRIBUTING.md](CONTRIBUTING.md) for setup, validation, and fixture
requirements, and the [Code of Conduct](CODE_OF_CONDUCT.md) for community
expectations. Report vulnerabilities privately using [SECURITY.md](SECURITY.md).

## License

MIT; see [LICENSE](LICENSE).
