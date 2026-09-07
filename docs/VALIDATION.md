# Validation

## macOS support (2026-09-07)

Host: Apple Silicon, macOS 27.0 build 26A5425a, Rust 1.97.1.

- Default, serde-only, and all-feature tests passed, including the full offline MFT fixture.
  The all-feature library suite contains 150 passing unit tests; live tests skip
  without an explicitly selected volume.
- Strict Clippy passed for all targets/features in the root and differential harness.
- The differential harness passed all 154 library tests, including 11 comparisons
  against the pinned upstream parser.
- All-feature library checks passed for Intel macOS, Linux x86_64, and Windows
  x86_64 MSVC. These are compile checks, not runtime tests on those platforms.
- All-feature rustdoc passed with warnings denied; formatting and diff checks passed.
- `cargo package --allow-dirty --offline` packaged and rebuilt the source locally.
- The user ran the compiled read-only hardware test with `sudo` in Terminal:
  `/Volumes/ESD-ISO` successfully resolved to the raw device for `/dev/disk6s1`.
  It validated 1,100 serial records, 69 chunks under both parallel schedulers
  with four workers, root-record rereading, independent cursors, and MFT stream
  ownership after dropping the original volume. An earlier explicit
  `/dev/rdisk6s1` run also passed. The volume remained mounted read-only.
- The device reports 512-byte logical blocks and 4,096-byte NTFS clusters.
  A deterministic fixture test covers simulated 4 KiB device alignment,
  bootstrap reads, and records crossing block boundaries; physical 4Kn media
  and older macOS releases have not been tested here.

The initial buffered-device open failed with `EBUSY` even under `sudo`. The
final macOS backend selects `/dev/rdisk...` and queries `DKIOCGETBLOCKSIZE`
instead. Hardware validation did not unmount the volume, write to it, or change
device permissions. The CI matrix includes a native macOS runner.

## Split validation (2026-09-06)

Validated with Rust 1.98.1 on Windows.

- Both crates: tests and doctests passed; strict Clippy over all targets/features passed.
- Both crates: `cargo package --allow-dirty --offline` packaged and compiled successfully.
- ntfs-mft: Windows and Linux documentation built with `RUSTDOCFLAGS=-D warnings`.
- ntfs-mft: Linux library passed `cargo clippy --lib --target x86_64-unknown-linux-gnu -- -D warnings`.
- Cross-repository harness: compiled and ran; all three live-volume checks explicitly
  skipped because the process was not elevated. Live NTFS parity is not verified here.
- Linux all-target cross-check reached Criterion's `alloca` build dependency but
  could not proceed because `x86_64-linux-gnu-gcc` is absent on this Windows host.
  Native Ubuntu CI is configured; Linux runtime tests have not been run locally.
- Retained journal, FSCTL MFT, USN parser, public types, Filetime, volume, privilege,
  live path entry and low-level live resolver implementation files are unchanged
  from source commit `0e57c8eed844b22e3b261a6823910932b3ad15ef`.

The raw reader keeps its existing algorithms. Extraction changes are crate/error
renaming, formatting, and equivalent fixed-size byte chunk iteration for current
Clippy. Shared support is trimmed to the new scope. Timestamp conversion tests
now run independently of device access. Cross-crate adapters compare file IDs
through numeric accessors and copy FILE_ATTRIBUTE bits rather than record-header flags.

These results describe the extraction checks before the initial GitHub push. No crates.io publication was performed.
