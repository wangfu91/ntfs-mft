# Changelog

## 0.1.0 (unreleased)

Initial public release.

- Read NTFS Master File Tables directly from volumes on Windows, Linux, and
  macOS using read-only I/O.
- Scan live volumes serially or in parallel, including fragmented `$MFT` data.
- Parse extracted logical `$MFT` streams from files, byte buffers, and custom
  `Read + Seek` sources.
- Expose record metadata including names, timestamps, sizes, file attributes,
  hard links, alternate data streams, and parent references.
- Preserve lossless UTF-16 names and complete NTFS file references.
- Report record corruption and metadata completeness when parsing logical MFT
  streams.
