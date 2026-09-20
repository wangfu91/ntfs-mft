# Changelog

## 0.1.0 (unreleased)

Initial public release.

- Reject oversized nonresident metadata before allocation, truncated volume
  reads, and overflowing data-run offsets.
- Read FILE records across fragmented extents and reject incomplete MFT
  bootstrap metadata instead of returning a shortened scan.
- Skip malformed attributes and continuation-only data sizes in raw entry
  construction, preserving unknown metadata.
- Group attribute-list references with indexed lookup to avoid quadratic work.
- Reject timestamps before the FILETIME epoch before rounding to ticks.
- Propagate scan errors from the serial-read example and fail benchmark runs
  instead of recording failed scans as successful timing samples.
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
