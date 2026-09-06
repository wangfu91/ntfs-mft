# Parser differential tests

The comparison implementation is `omerbenamram/mft` at immutable revision
`dc36db7d3b83d68c0eecb75225c9955cfd7b2f2d`, with its default CLI features disabled.
The existing attributed fixture and independent expected-value tests are unchanged.
Neither parser is an oracle. The adapters consume each parser's results independently;
byte-authored synthetic records supply separate expectations.

## Comparison contract

Run from the repository root after restoring the fixture's Git LFS object:

```text
cargo test --manifest-path differential-tests/Cargo.toml --locked --lib differential_tests
cargo clippy --manifest-path differential-tests/Cargo.toml --locked --all-targets --all-features -- -D warnings
```

After fetching dependencies once, add `--offline` to run without network access.
The test filter runs only differential cases and never attempts device access.
CI runs these commands on Windows and Linux. The fixture size assertion fails
clearly if an LFS pointer is present; never silently skip a missing fixture.

This package has `publish = false`, its own lockfile, and is excluded from the
main Cargo package. The main manifest has no upstream comparison dependency;
ordinary and packaged-source `cargo test` still run the independent fixture tests.
Cargo removes Git-only dev-dependencies from published manifests, which is why
the pinned comparison lives in a separate package instead of the main manifest.

The harness compiles `../src/lib.rs` directly. Only its build script enables
`ntfs_mft_differential`, loading this package's private tests into `raw_mft`.
The main crate recognizes that cfg name for linting but does not enable it,
including under `--all-features`. There is no exported parser or test API.
The production dependency declaration block is mirrored and checked against the
root manifest at build time; update it when root dependencies change. The separate
lockfile can resolve different compatible transitive versions and must be reviewed
when refreshed. No sibling task's unmerged implementation is needed.

Each parser receives its own copy of the original, pre-USA-fixup bytes and the same
record-slot number. No volume is opened. The seam is `(number: u64, bytes: &[u8])`;
it needs no proposed offline API and imposes no public API changes on that task.

Compare header sequence, slot number, base reference (including sequence), link
count, and the low four record flag bits retained by upstream. Compare every
overlapping SI, FN and DATA attribute in physical order, including its offset and
instance ID. This retains duplicate names, namespaces, parents and stream extents;
there is no sorting, deduplication, primary-name selection, or extent merging in
the low-level adapter. Non-overlapping attribute payloads are not compared, but
their headers must still be traversable. No upstream attribute errors are discarded.

Names are exact UTF-16 units from the parsed views, not lossy display strings.
Parent and base references retain all 48 record bits and 16 sequence bits. SI and
FN timestamps remain separate. DATA residency comes from the attribute form;
resident size is payload length, while nonresident size, allocated length,
initialized length and VCN bounds come from the nonresident header. There is no
invented allocated-size field for resident DATA. A continuation extent is compared
as stored, without treating its size fields as independently authoritative totals.

The real fixture covers 13,068 FILE records, 13,064 SI attributes, 18,506 FN
attributes and 12,089 DATA attributes, plus 20 zero slots. These inventory counts
are regression coverage guards, not independent evidence of correct decoding.
The existing `fixture_tests.rs` provides the independent fixture expectations.

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

For any new mismatch, retain the original bytes, identify record/attribute offset
and instance, and inspect the relevant fields after USA restoration. Consult the
format semantics and each implementation's documented contract before classifying
it. Add a narrow, byte-explained regression; do not skip a record, drop failed
attributes, or regenerate expected data from either implementation. Production
parser fixes require separate scope approval.

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

## Limits

Agreement on a shared corpus does not establish parser correctness for every NTFS
version, sector geometry or corrupt input. This is not a fuzzing campaign. It does
not validate boot-sector discovery, live-volume consistency, raw-device I/O,
external attribute payloads, runlist correctness, cross-record enrichment, full
stream reconstruction, path resolution, or serial/parallel scanning and scheduling.
No performance claim is made. Windows/Linux display-string policies are not
equated by comparing raw UTF-16 views.

Integration conflicts should be limited to the development harness, dependency
metadata and the test-module declaration. The offline API and export tasks should
not depend on these test-only normalized types or assume sibling changes exist.
