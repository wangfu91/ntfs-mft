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
[implementation](docs/raw-mft-read.md), and [extraction provenance](docs/EXTRACTION.md).
Performance notes retain the original benchmark environment variable names.

## License

MIT; see [LICENSE](LICENSE).
