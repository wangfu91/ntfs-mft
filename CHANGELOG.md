# Changelog

## 0.1.0 (unreleased)

- Extract raw NTFS MFT reading from usn-journal-rs as an independent crate.
- Preserve serial/parallel scans, metadata parsing, snapshot/historical paths,
  and Windows/Linux/macOS read-only volume access.
- Add native macOS mount lookup, raw disk reads with device-block alignment,
  and macOS CI coverage.
- Expose MftError and MftResult and root-level raw reader exports.
