# Parser differential tests

The comparison parser is `omerbenamram/mft`, pinned to
`dc36db7d3b83d68c0eecb75225c9955cfd7b2f2d` with CLI features disabled.
Neither parser is an oracle: independent fixture expectations and byte-authored
records provide separate checks.

```sh
git lfs pull
cargo test --manifest-path differential-tests/Cargo.toml --locked --lib differential_tests
cargo clippy --manifest-path differential-tests/Cargo.toml --locked --all-targets -- -D warnings
```

Run from the repository root; add `--offline` after fetching dependencies.
The filter runs only differential tests, without device access. Missing LFS
contents fail the size check. CI runs on Windows, Linux, and macOS.

## Harness and comparison contract

This unpublished workspace package is excluded from the main Cargo package. It
compiles `../src/lib.rs`; its build script alone enables `ntfs_mft_differential`.
No comparison API or upstream dependency is exposed by the main crate. Review
transitive version changes when updating the workspace lockfile.

Each parser receives a separate copy of the original USA-protected bytes and
the same physical slot number. Comparisons retain:

- Header sequence, slot, full base reference, link count, and the low four flag
  bits supported by upstream.
- Every overlapping SI, FN, and DATA attribute in physical order, including
  offsets, instance IDs, duplicate names, namespaces, and extents. Other payloads
  are not compared, but their headers must remain traversable.
- Exact UTF-16 name units and full parent/base references. A missing DATA name
  view is empty only when its declared length is zero; malformed named views fail.
- Separate SI/FN timestamps and DATA residency. Resident size is payload length;
  nonresident sizes, initialized length, and VCN bounds come from the header.
  Continuation size fields are compared as stored, not treated as stream totals.

No failed attributes are discarded or names sorted, deduplicated, or ranked by
these low-level comparisons. The corpus contains 13,068 FILE records, 13,064 SI,
18,506 FN, and 12,089 DATA attributes, plus 20 zero slots. Counts guard coverage;
`src/raw_mft/tests/fixture.rs` supplies independent expected metadata.

## Investigated differences

* **Timestamp precision:** upstream `src/utils.rs::windows_filetime_to_timestamp`
  explicitly divides ticks by 10 before converting to microseconds. Only that
  exact truncation is applied to the comparison copy. Local raw ticks are retained
  and independently asserted for both SI lengths (48/72), FN, the 1601 epoch,
  just before the Unix epoch, and nonzero sub-microsecond digits. No numeric
  tolerance or string/date formatting is used. Values beyond upstream's timestamp
  range are outside the common successful-decoding domain.
* **Primary name:** local namespace score prefers combined Win32/DOS over Win32;
  upstream `find_best_name_attribute` keeps the first Win32 or combined name.
  Both attribute orders are tested separately from field equality. No changes to
  either selection policy are implied by a difference.
* **USA corruption/truncation:** local parsing rejects a mismatched trailer or a
  record shorter than the USA sector count. Upstream `src/ntfs.rs` documents
  best-effort restoration and returns `valid_fixup = false` for both, when enough
  header/USA bytes are present. Short header/USA inputs error in both. An upstream
  `Ok` is never accepted as a valid record without checking the fixup flag.
* **Zero slots:** local parser rejects them; upstream returns an invalid/zero
  header and no fixup status. Each outcome is asserted explicitly.
* **Malformed attribute lengths:** zero and out-of-record lengths stop the local
  attribute iterator; upstream yields an error. Test success here describes these
  different contracts, not full-record validation by the local parser.
* **Unknown header flags:** local header preserves all bits; upstream uses
  `from_bits_truncate` with mask `0x000f`. A byte-authored `0x800f` case asserts
  both raw results; masking the comparison is not an assertion that other bits
  are zero or invalid.
* **Embedded NUL in a stream name:** local counted view retains all declared
  units; upstream attribute-header parsing explicitly truncates at the first NUL.
  A hostile-input case asserts both exact results. It is not claimed to be a
  normal Windows-created name. Lone surrogates in FN are compared before either
  implementation converts them to display strings.

For a mismatch, retain the bytes and record/attribute location, inspect the
restored fields and both parser contracts, then add a byte-explained regression.
Do not skip records or regenerate expectations from either parser.

## Stream coverage and limits

The fragmented-stream test reverses every 512-byte fixture sector on a simulated
device, forcing each 1,024-byte record across extents. It checks original bytes,
the low-level comparisons, public `MftStream` metadata, all FILE/zero-slot
outcomes, and backwards reads. Selected names must occur in upstream attributes;
selection policy and platform string conversion remain implementation-specific.

These comparisons do not validate bootstrapping, real-device I/O, live consistency,
external payloads, runlist discovery, cross-record enrichment, paths, or volume
scan scheduling. They are neither fuzzing nor performance evidence. See the
[live stream test](../docs/mft-stream-validation.md) for separate device coverage.

## Semantics references

* [Microsoft FILETIME](https://learn.microsoft.com/en-us/windows/win32/api/minwinbase/ns-minwinbase-filetime):
  100ns intervals since 1601 UTC, distinct from filesystem update frequency.
* [Microsoft FILE_RECORD_SEGMENT_HEADER](https://learn.microsoft.com/en-us/windows/win32/devnotes/file-record-segment-header):
  sequence, base reference, defined in-use/index flags, and USA protection.
* [Microsoft ATTRIBUTE_RECORD_HEADER](https://learn.microsoft.com/en-us/windows/win32/devnotes/attribute-record-header):
  counted names, residency and stream length/VCN fields.
* [Pinned upstream source](https://github.com/omerbenamram/mft/tree/dc36db7d3b83d68c0eecb75225c9955cfd7b2f2d/src):
  `utils.rs`, `entry.rs`, `ntfs.rs`, `attribute/header.rs`, and `utf16.rs` explain
  the implementation-specific behaviors above.
