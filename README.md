# ntfs-mft

Read-only raw NTFS Master File Table parsing and scanning for Windows and Linux.

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
    let mft = volume.raw_mft()?;
    for entry in mft.try_iter()?.take(20) {
        println!("{}", entry?.file_name.to_string_lossy());
    }
    Ok(())
}
```

## Extracted MFT streams

`MftStream` reads a standalone logical `$MFT` file using ordinary file permissions.
It accepts any `Read + Seek` source, including borrowed memory:

```rust,no_run
use ntfs_mft::{MftResult, MftStream, MftStreamOptions};
use std::io::Cursor;

fn main() -> MftResult<()> {
    let options = MftStreamOptions::new(1024)?;
    let mut mft = MftStream::from_path("extracted.MFT", options)?;
    for record in mft.iter_records() {
        match record {
            Ok(record) => println!("{} {:?}", record.entry, record.metadata),
            Err(error) => eprintln!("{error}"),
        }
    }
    let bytes = std::fs::read("extracted.MFT")?;
    let mut memory = MftStream::from_reader(Cursor::new(bytes.as_slice()), options)?;
    let root = memory.read_record(5)?;
    println!("{}", root.entry.file_name.to_string_lossy());
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

## Scope and limitations

- NTFS only; ReFS is rejected. USN journal APIs and Windows FSCTL enumeration
  remain in [usn-journal-rs](https://github.com/wangfu91/usn-journal-rs).
- Windows raw volume access requires Administrator privileges. Linux requires
  read permission for the backing block device. All device opens are read-only.
- A live scan is not an atomic filesystem snapshot; concurrent changes can affect results.
- Attribute-list enrichment is one level deep. Historical paths are best effort.
- Snapshot paths are the default; Windows live fallback is explicitly opt-in.

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

On Linux, pass an NTFS mount point or `/dev/...` device instead of `C`.
Device tests skip when privileges or a suitable volume are unavailable.
See [cross-repository tests](integration-tests/README.md),
[offline parser differential tests](differential-tests/README.md),
[implementation](docs/raw-mft-read.md), and [extraction provenance](docs/EXTRACTION.md).
Performance notes retain the original benchmark environment variable names.

## License

MIT; see [LICENSE](LICENSE).

## Test fixture attribution

Our offline correctness tests use the unmodified samples/MFT fixture from
[omerbenamram/mft](https://github.com/omerbenamram/mft), copyright (c) 2019
Omer Ben-Amram. Upstream offers MIT/Apache-2.0 licensing; this copy uses the MIT
option and retains the [upstream license](tests/fixtures/omerbenamram-mft/LICENSE-MIT).
See [fixture provenance and test scope](tests/fixtures/omerbenamram-mft/README.md)
for the pinned commit, checksum, and coverage. Run these tests with cargo test;
no NTFS volume or Administrator privileges are needed for the fixture tests.

The MFT fixture is stored with Git LFS. Install Git LFS, then run
`git lfs install` and `git lfs pull` after cloning and before building or
testing. CI checks out LFS objects automatically.
