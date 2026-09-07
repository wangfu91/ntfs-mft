# MFT stream validation

## Live Windows test

I ran the `mft_byte_reader` integration test in an independent administrator
PowerShell with `USN_TEST_DRIVE=C` on Windows 11. It passed with a 3,420,454,912-byte
MFT, 1,024-byte records, and 28 valid records among the first 32 slots.

The test covered logical length, EOF, independent handles, ownership after
dropping the original volume, root-record lookup, and iteration. It was a smoke
test, not a full-volume comparison or benchmark. The same bootstrap failed in
Codex's inherited MSIX Silo; see [Windows access](windows-raw-volume-access.md).

## Offline coverage

`cargo test` covers fragmented extents (including split FILE records), sparse
and uninitialized ranges, EOF, arbitrary offsets, short reads, failing I/O,
overflow rejection, and original USA bytes.

The [differential harness](../differential-tests/README.md) reverses all fixture
sectors on a simulated device and compares the mapped byte reader and public
iterator with the pinned parser: 13,068 FILE records, 20 zero-slot errors,
metadata comparisons, and backwards random reads.

These tests do not replace live-device validation. A skipped device test is not
a successful hardware check. See [validation scope](VALIDATION.md).
