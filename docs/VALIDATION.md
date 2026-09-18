# Validation

These are recorded local results, not the current CI status.

## Scope cleanup phases (2026-09-18, Windows)

- Export removal: default tests, all-target Clippy, rustdoc with warnings denied,
  both test harnesses, formatting, and offline package rebuild passed. The
  sparse-fixture JSON parser and binary fixtures were retained.
- Lockfile policy: a fresh tracked-file copy without a root Cargo.lock resolved
  dependencies and compiled all targets offline. Both harnesses still passed
  with their tracked lockfiles and `--locked`.
- Live-test consolidation: 183 library tests, offline integration tests, two
  configuration tests, and doctests passed. Four root live tests and one parity
  test were explicitly reported as ignored. All 11 differential comparisons and
  Clippy for the library and both harnesses passed.
- Explicitly selecting live tests with missing or malformed configuration
  produced the expected failures; these negative checks did not access a device.
- Formatting, warning-free rustdoc, package rebuild, and the downstream
  SpaceRover raw-mft consumer compile check passed after all phases.
- Linux library cross-compilation passed. Linux all-target checking was blocked
  by missing x86_64-linux-gnu-gcc for a C build dependency. Native Linux/macOS and
  physical NTFS/ReFS execution were not performed in this run.

## Path API removal (2026-09-18, Windows)

- Default and all-feature test suites passed (187 and 194 library tests,
  respectively), along with integration tests, export example tests, and doctests.
- All-target/all-feature Clippy, rustdoc with warnings denied, and formatting passed.
- All remaining examples built in release mode with all features enabled.
- SpaceRover's spacerover-scan-advanced crate compiled against this local library
  with its raw-mft feature enabled (offline, locked dependency resolution).
- The independent Windows API harness and its Clippy checks passed; both live
  parity checks explicitly skipped because the process was not elevated.
- All 11 pinned parser differential tests and differential all-feature Clippy passed.
- `cargo package --locked --allow-dirty` packaged and rebuilt the library.
  Fixtures were hydrated. Linux/macOS execution and elevated live-volume behavior
  were not validated in this run.

## macOS and sparse fixtures

Host: Apple Silicon, macOS 27.0 build 26A5425a, Rust 1.97.1.

- The all-feature suite passed 155 library tests plus integration tests and
  doctests; the differential harness passed 159 library tests.
- Strict Clippy passed for both packages, along with formatting, fixture hash
  verification, and local Cargo packaging/rebuild.
- Before the fixture addition, all-feature library checks passed for Intel macOS,
  Linux x86_64, and Windows x86_64 MSVC; rustdoc passed with warnings denied.
  Cross-checking the added test code was blocked by a missing Linux cross-compiler
  and an uncached Windows test dependency. Native CI still needs to verify it.
- On the Windows installation USB from which the sparse fixture was sourced, the privileged live test passed through both the mount point
  and raw device: 1,100 serial records, 69 chunks with both schedulers/four workers,
  root rereading, independent cursors, and stream ownership. The volume stayed
  mounted read-only. Its device blocks are 512 bytes and NTFS clusters 4,096 bytes.
- Simulated 4 KiB alignment is tested; physical 4Kn media and older macOS releases
  remain unverified. Ordinary device tests skip without a selected source.

## Original crate split

On Windows with Rust 1.98.1, both split crates passed tests, doctests, strict
Clippy, and local packaging. NTFS library documentation built for Windows/Linux,
and Linux library Clippy passed. The cross-repository harness ran but skipped
all three live tests without elevation, so it did not establish live parity.
Linux all-target checking lacked `x86_64-linux-gnu-gcc`.

No crates.io publication was part of these checks.
