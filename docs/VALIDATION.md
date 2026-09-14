# Validation

These are recorded local results, not the current CI status.

## macOS and captured fixtures

Host: Apple Silicon, macOS 27.0 build 26A5425a, Rust 1.97.1.

- The all-feature suite passed 155 library tests plus integration tests and
  doctests; the differential harness passed 159 library tests.
- Strict Clippy passed for both packages, along with formatting, fixture hash
  verification, and local Cargo packaging/rebuild.
- Before the fixture addition, all-feature library checks passed for Intel macOS,
  Linux x86_64, and Windows x86_64 MSVC; rustdoc passed with warnings denied.
  Cross-checking the added test code was blocked by a missing Linux cross-compiler
  and an uncached Windows test dependency. Native CI still needs to verify it.
- On the captured-fixture source USB, the privileged live test passed through both the mount point
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
