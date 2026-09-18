# Changelog

## 0.1.0 (unreleased)

- Consolidate live-device test configuration and duplicate parity/ReFS checks.
  Device tests are ignored by default and fail on missing prerequisites or
  runtime errors when explicitly selected.

- Align CI and contributor commands with the untracked root lockfile; retain
  locked dependency resolution for the standalone test harnesses.

- Remove optional serialization/export APIs, JSONL tooling, and schema validation
  dependencies; consumers own their serialization format.

- Remove path resolvers, in-memory path indexes, historical reconstruction, and
  Windows live-path fallback before publication. Path/tree policy belongs to
  consumers; raw names, identities, parent references, and links remain available.
- Stream unused-record previews without constructing a historical path index.

- Declare Rust 1.88 as the minimum supported version and add MSRV CI coverage.
- Add contribution, security-reporting, and community guidance for the public release.

- Extract raw NTFS MFT reading from usn-journal-rs as an independent crate.
- Preserve serial/parallel scans, metadata parsing,
  and Windows/Linux/macOS read-only volume access.
- Add native macOS mount lookup, raw disk reads with device-block alignment,
  and macOS CI coverage.
- Expose MftError and MftResult and root-level raw reader exports.
