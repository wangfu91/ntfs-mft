# ntfs-mft

Read-only NTFS Master File Table parsing and scanning for Windows, Linux, and macOS.

Read an NTFS volume or an extracted `$MFT` to inspect names, timestamps, sizes,
hard links, alternate data streams, and file attributes. Volume scans support
fragmented MFTs and serial or parallel processing. Names retain their original
UTF-16, and file references retain record and sequence numbers.

## Installation

Version 0.1.0 is being prepared for its first crates.io publication. Until then,
clone this repository and use a path dependency:

```toml
[dependencies]
ntfs-mft = { path = "../ntfs-mft" }
```

After publication, use `ntfs-mft = "0.1"`. Development and CI use the latest
stable Rust; no minimum supported Rust version is declared.

## Parse an extracted MFT

Use `MftStream` with a logical `$MFT` file and its record size. This requires
ordinary file permissions and works on every supported OS:

```rust,no_run
use ntfs_mft::{MftResult, MftStream, MftStreamOptions};

fn main() -> MftResult<()> {
    // Use the source volume's record size; 1024 is common, but not universal.
    let options = MftStreamOptions::new(1024)?;
    let mut mft = MftStream::from_path("extracted.MFT", options)?;
    for result in mft.iter_records() {
        let record = result?;
        println!("{} {:?}", record.entry, record.metadata);
    }
    Ok(())
}
```

The stream includes every slot, including unused and extension records. Invalid
records return per-slot errors; the example stops at the first error. To continue
past recoverable parse errors, handle each `Result` instead of using `?`.
Keep `metadata`, `diagnostics`, and `external_payloads` with exported entries:
an empty field may mean unknown, not absent. See the
[stream contract](docs/offline-mft.md) for input requirements and error handling.

## Scan a volume

```rust,no_run
use ntfs_mft::{MftResult, Volume};

fn main() -> MftResult<()> {
    #[cfg(windows)]
    let volume = Volume::from_drive_letter('C')?;
    #[cfg(target_os = "linux")]
    let volume = Volume::from_mount_point("/media/user/windows")?;
    #[cfg(target_os = "macos")]
    let volume = Volume::from_mount_point("/Volumes/Windows")?;

    let mft = volume.raw_mft()?;
    for entry in mft.scan_entries()?.take(20) {
        println!("{}", entry?.file_name);
    }
    Ok(())
}
```

Volume scans default to record 24 onward, exclude bitmap-unused slots, and skip
malformed and extension records. Scan I/O failures end iteration. Metadata enrichment
is best effort; a successful scan does not establish complete metadata.

See the [API guide](docs/api-guide.md) for parallel scans, selection, metadata
options, and error contracts.

## Limits and access requirements

- All device access is read-only. Windows requires Administrator privileges and
  a launch context permitting raw-volume I/O; see
  [Windows access requirements](docs/windows-raw-volume-access.md). Linux and macOS
  require read permission for the backing device; see
  [macOS access and validation](docs/macos-raw-volume-access.md).
- NTFS only; ReFS is rejected. Live scans are not atomic filesystem snapshots.
- Attribute-list enrichment follows one level. `MftStream` resolves only
  resident attribute lists and cannot read external attribute payloads or general
  file contents, including when backed by a volume.
- Full paths and directory trees belong to the consumer. USN journal APIs are
  provided separately by [usn-journal-rs](https://crates.io/crates/usn-journal-rs).

## Development and documentation

Tests and packaging require hydrated [Git LFS](https://git-lfs.com/) fixtures.
Run `git lfs pull` after cloning; LFS pointer files are not usable fixtures.

- [Contributing](CONTRIBUTING.md): setup, validation, live-device tests, and benchmarks.
- [Implementation guide](docs/raw-mft-read.md): bootstrapping, record parsing, and parallel scans.
- [Differential tests](differential-tests/README.md) and [Windows API parity tests](integration-tests/README.md).
- [Release checklist](docs/releasing.md) and [changelog](CHANGELOG.md).
- [Security reporting](SECURITY.md) and [Code of Conduct](CODE_OF_CONDUCT.md).

## License

MIT; see [LICENSE](LICENSE). Binary fixtures have their own provenance and notices
under [`tests/fixtures`](tests/fixtures).
