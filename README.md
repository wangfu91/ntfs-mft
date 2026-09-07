# ntfs-mft

Read-only raw NTFS Master File Table parsing and scanning for Windows, Linux, and macOS.

This library reads the boot sector and fragmented `$MFT` streams directly,
validates FILE records and update sequence arrays, and exposes timestamps,
names, sizes, hard links, alternate data streams, and file attributes. It includes
serial and parallel scans, snapshot path resolution, and experimental historical
path reconstruction.

## Local use

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

## Extracted MFT streams

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

For direct volume input without exporting first, pass `raw.byte_reader()?` to
`MftStream::from_reader`, where `raw` is an existing `RawMft`. The adapter handles
fragmented extents and exposes logical `$MFT::$DATA` bytes through `Read + Seek`.
Use `raw.file_record_size()` for the parser options. Raw-volume permissions and
live-scan consistency limitations still apply.

## Scope and limitations

- NTFS only; ReFS is rejected. USN journal APIs and Windows FSCTL enumeration
  remain in [usn-journal-rs](https://github.com/wangfu91/usn-journal-rs).
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
timestamps, OS name encodings, metadata completeness, and known parser limits.

```text
cargo run --example raw_mft_export --features jsonl -- file extracted.MFT 1024
```

## Development

```text
cargo build
cargo test
cargo test --manifest-path differential-tests/Cargo.toml --locked --lib differential_tests
cargo clippy --all-targets --all-features -- -D warnings
cargo package
cargo run --example raw_mft_serial_read -- C
cargo run --example raw_mft_parallel_chunks -- C
cargo run --example deletion_forensic -- C
cargo bench --bench raw_mft
cargo bench --bench raw_mft_ingest
cargo bench --features windows-examples --bench path_resolver
```

On Linux and macOS, pass an NTFS mount point or `/dev/...` device instead of `C`.
Device tests skip when privileges or a suitable volume are unavailable.
See [cross-repository tests](integration-tests/README.md),
[offline parser differential tests](differential-tests/README.md),
[implementation](docs/raw-mft-read.md), and [extraction provenance](docs/EXTRACTION.md).

## License

MIT; see [LICENSE](LICENSE).

## Test fixture attribution

Our offline correctness tests use the unmodified samples/MFT fixture from
[omerbenamram/mft](https://github.com/omerbenamram/mft), copyright (c) 2019
Omer Ben-Amram. Upstream offers MIT/Apache-2.0 licensing; this copy uses the MIT
option and retains the [upstream license](tests/fixtures/omerbenamram-mft/LICENSE-MIT).
See [fixture provenance](tests/fixtures/omerbenamram-mft/README.md) for the pinned
commit, checksum, and coverage. Fixture tests require no device or elevation.

I extracted the [ESD-ISO fixture](tests/fixtures/esd-iso/README.md) from my Windows
installation USB. Its boot sector, MFT, and fragmented MFT bitmap cover offline
bootstrapping and serial/parallel scans.

Both fixtures use Git LFS. Install Git LFS, then run `git lfs install` and
`git lfs pull` after cloning and before building or testing. CI checks out LFS
objects automatically.
