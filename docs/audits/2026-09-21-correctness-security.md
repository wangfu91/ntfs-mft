# Correctness and security audit — 2026-09-21

Audited revision: `fb3620f3e87731edf0f3cda65464e14fe91afdf3`.
The accompanying fixes address the findings in that revision.

The review covered production Rust code, platform I/O and FFI, parser and
metadata boundaries, serial/parallel execution, examples, test and benchmark
helpers, and CI configuration. An independent baseline review and focused
parser/architecture reviews informed the findings. Inputs were synthetic or
existing licensed fixtures; no personal disk capture was created.

## Security findings

All three findings are rated **medium**: a consumer must select an
attacker-controlled NTFS image/volume or encounter source truncation. No remote
service exposure, arbitrary code execution, privilege escalation, or filesystem
write was established.

| ID | Finding and original impact | Resolution and evidence |
| --- | --- | --- |
| S1 | `reader::read_nonresident` allocated the untrusted `data_size` before checking run coverage. `u64::MAX` caused a capacity-overflow panic on 64-bit; smaller enormous values could exhaust memory. | Enforce a 64 MiB limit per materialized metadata payload, validate coverage/offsets before allocation or I/O, and reserve fallibly. `read_nonresident_rejects_untrusted_allocation_sizes` failed before the fix and passes afterward; coverage-before-I/O is also tested. CWE-789. |
| S2 | `VolumeReader::read` accepted a positive refill ending before the requested intra-sector position. Availability subtraction underflowed; release builds could return stale bytes from the cache. | Check coverage after refill and return `UnexpectedEof` before arithmetic/copying. A prefilled-cache, short-refill regression reproduced the original panic and verifies unchanged output/cursor after the fix. Safe Rust prevented arbitrary memory access. CWE-191/CWE-20. |
| S3 | Raw nonresident attribute-list grouping linearly searched previous groups for every distinct reference, causing quadratic CPU work before extension I/O. | Use a randomized `HashMap` index while retaining first-seen group order and per-record reference order. A 20,000-reference regression checks those semantics. Complexity is established from source, not a timing benchmark. CWE-407. |

The metadata limit applies to `$MFT::$BITMAP` and nonresident `$ATTRIBUTE_LIST`
payloads, not the entire MFT or ordinary file sizes. Bootstrap returns an error
when it is exceeded; best-effort entry enrichment retains unknown metadata.

## Correctness findings

| ID | Finding | Resolution |
| --- | --- | --- |
| C1 | Raw record readers mapped only the first byte and assumed the whole FILE record was physically contiguous. Valid records can span nonadjacent extents when clusters are smaller than records. | Map complete record coverage, retain the borrowed-buffer path for contiguous records, and assemble split records before fixup. Synthetic tests compare single-record, serial, rich/lean parallel and bootstrap-extension reads with logical stream parsing; sparse/missing tails and offset overflow are checked. |
| C2 | Raw base builders bypassed the shared attribute-shape checks. Invalid named DATA could become unnamed data; invalid resident lengths and filename namespaces became observed metadata. | Apply the shared allocation-free shape predicate before raw dispatch. Synthetic regressions failed before the fix and verify later valid metadata survives and malformed fields stay unknown. |
| C3 | Raw builders interpreted continuation DATA segment sizes as authoritative and suppressed missing-data enrichment. | Only initial DATA segments supply sizes, matching logical-stream and extension selection. Test continuation-only and continuation-before-initial records. |
| C4 | Bootstrap discarded malformed attribute/list evidence and failed runlist decoding, allowing a successful shortened MFT map. Saturated capacity arithmetic also admitted impossible scan ranges. | Propagate structural discovery errors, reject incomplete attribute lists and malformed required runlists, validate declared DATA coverage, checked extent capacity and the 48-bit record space. Retain valid split streams and an absent optional bitmap. Synthetic bootstrap regressions cover these cases. |
| C5 | Data-run LCN accumulation narrowed a positive `i128` to `u64` without checking overflow. | Reject out-of-range LCNs with `u64::try_from`. A run chain crossing `u64::MAX` failed the regression before the fix and is now rejected. Downstream checks limited demonstrated security impact. |
| C6 | Relative volume seeks negated `i64::MIN`, causing a debug panic. | Use `checked_add_signed`; test underflow and a valid seek from `2^63` back to zero. |
| C7 | The serial-read example printed terminal scan errors but exited successfully. | Propagate the error to its existing nonzero-exit handler. |
| C8 | Sub-100 ns times just before the FILETIME epoch could round into a valid zero value on high-resolution platforms. | Validate the epoch boundary before truncating to FILETIME ticks; cover the conversion arithmetic with synthetic durations. |
| C9 | Benchmark helpers replaced failed scans with successful-looking empty/partial results, allowing invalid timing samples. | Propagate serial scan errors and fail timed benchmark operations instead of substituting defaults. No benchmark performance claim is made by this audit. |

## Validation and limits

Validated locally on Windows with Rust/Cargo 1.98.1 and hydrated Git LFS fixtures:

| Check | Result |
| --- | --- |
| `cargo fmt --all -- --check` | Passed. |
| `cargo test --locked` | 208 unit tests, 26 integration tests and 12 doctests passed; four live-device tests remained ignored. |
| `cargo clippy --locked --all-targets -- -D warnings` | Passed, including examples and benchmarks. |
| `cargo doc --locked --no-deps` | Passed. |
| `cargo test --manifest-path differential-tests/Cargo.toml --locked --lib differential_tests` | All 11 differential tests passed against the pinned independent parser. |
| `cargo clippy --manifest-path differential-tests/Cargo.toml --locked --all-targets -- -D warnings` | Passed. |
| `cargo test --manifest-path integration-tests/Cargo.toml --locked` | Two decoder tests passed; the live Windows API parity test remained ignored. |
| `cargo clippy --manifest-path integration-tests/Cargo.toml --locked --all-targets -- -D warnings` | Passed. |
| `cargo package --locked --allow-dirty --offline` | Package creation and verification build passed. The original command rejected uncommitted changes; the online retry failed with Windows TLS `SEC_E_NO_CREDENTIALS`. Offline verification used cached locked dependencies. |
| `python tests/fixtures/sparse-volume-fixture/verify.py` | Passed: four ranges, 1,280 record slots and 19 FILE records. |
| `git diff --check` | Passed. |

The baseline had 187 passing unit tests; the fixes add 21 focused unit
regressions. No publication or live-volume benchmarks were performed.

Review established read-only device opens on all supported platforms, bounded
logical-record parsing, checked full extension identities, and names treated as
data. Live scans remain non-atomic; consumer path construction and output privacy
remain caller obligations. Best-effort raw enrichment still does not certify
complete metadata.

No live-device tests, native Linux/macOS execution, CI run, sustained fuzzing,
external dependency-advisory lookup, or performance benchmark was performed.
These remain separate evidence requirements. This audit is a source review and
regression exercise, not a guarantee that all defects have been eliminated.
