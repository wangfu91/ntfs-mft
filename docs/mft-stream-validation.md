# MFT stream validation

## Windows 11 C: live-volume smoke test

The user ran `mft_byte_reader-c673c4462d68deaf.exe --nocapture` in a separately
opened Administrator PowerShell with `USN_TEST_DRIVE=C` and supplied this result:

```text
Testing Windows volume C:
Opened volume; discovering raw MFT
Discovered raw MFT; reading root through existing API
Opening first MFT byte reader
Validated live MFT byte reader: 3420454912 bytes, 1024-byte records, 28 valid records in first 32 slots
test volume_byte_reader_feeds_iterator_and_owns_independent_handle ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.46s
```

This is user-supplied evidence of the live smoke test, not a complete-volume
comparison or a benchmark. It covers logical length, EOF, independent handles,
reader lifetime after dropping the original volume, root-record lookup and
iteration over the first 32 slots. D: is ReFS and is not an MFT validation target.

The same binary launched by the agent through `sudo --inline` failed in the
existing `RawMft` bootstrap before adapter construction. A separate PowerShell
script using .NET `FileStream` also failed to read C: via that launch path, with
Windows error 0x80070032. Those failures do not identify sudo itself as the cause;
the launch/execution environment remains a separate investigation. Speculative
changes to access flags and boot read sizes were reverted.

## Deterministic tests

`cargo test` includes offline API fixtures and simulated-volume adapter tests:
fragmented physical extents (including a split FILE record), sparse and
uninitialized ranges, logical EOF, arbitrary byte offsets, short reads, failing
seeks/reads, overflow rejection, and original USA byte preservation.

Strict Clippy and `cargo package --allow-dirty --offline` also pass. Device tests
in an unelevated environment skip; such skips do not count as live validation.
