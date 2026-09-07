# Changelog

## 0.1.0 (unreleased)

- Declare Rust 1.88 as the minimum supported version and add MSRV CI coverage.
- Add contribution, security-reporting, and community guidance for the public release.

- Extract raw NTFS MFT reading from usn-journal-rs as an independent crate.
- Preserve serial/parallel scans, metadata parsing, snapshot/historical paths,
  and Windows/Linux/macOS read-only volume access.
- Add native macOS mount lookup, raw disk reads with device-block alignment,
  and macOS CI coverage.
- Expose MftError and MftResult and root-level raw reader exports.
