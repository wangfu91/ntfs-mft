# Split validation (2026-09-06)

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
