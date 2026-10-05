# Release review — 2026-10-05

Baseline: `a973e53ead27b99fe4e48956def510cffc91e580`.
Documentation and release-guide fixes: `16eebec` and `90bd44c`.
Planning fix: `2de50dc`; optimized privilege-test repair: `9dabff1`;
upstream fixture privacy notice: `654d945`.
Host: Windows 11 Pro 10.0.26300, x86_64 MSVC, Rust/Cargo 1.98.1.

**Local Windows validation and packaging pass.** The planning defect is fixed,
including the approved change to the unpublished planning API. The upstream
fixture is retained with an explicit privacy caveat. Native Linux/macOS CI
evidence remains unavailable, and fixture rights/privacy caveats still apply
before publication. No repository visibility, crate publication, tag, push,
release or account setting was changed during this review.

## Changes made

- Shortened the README and compiled its examples along with the API and internal
  guides. Corrected offline-versus-device permissions, extension-I/O behavior,
  metadata completeness, and stale rustdoc links to the former default branch.
- Corrected macOS live-test instructions: explicitly selected tests fail when
  configuration is missing. Updated contribution templates to the current API.
- Separated historical validation from the current validation guide, preserving
  historical evidence. Documented package-specific rustdoc, workspace validation,
  archive inspection and the limits of a publish dry run.
- Corrected the release checklist's implication that both fixtures carry license
  grants. The library and upstream fixture have MIT notices; the Windows fixture
  has provenance and an explicit Microsoft-content rights caveat.
- Bounded and made parallel chunk planning fallible, migrated callers, and added
  synthetic regressions for oversized geometry and boundary behavior.
- Replaced optimizer-sensitive Windows privilege-test patching with a private
  token acquisition/query boundary. Tests now verify error propagation and
  exactly-once token cleanup with safe mock tokens in debug and release builds.
  Removed `injectorpp` and its two unused dependencies.

## Planning fix

### Finding

Static review identified a resource-exhaustion path in
`src/raw_mft/chunk_plan.rs` and `src/raw_mft/parallel/chunks.rs`.
Bootstrap validates arithmetic and the 48-bit record-number limit, but a small
input can claim a very large extent capacity without supplying those bytes.
Default planning used that capacity and eagerly created the complete chunk
vector before a scan read could fail at EOF. Missing allocation evidence kept
the claimed ranges as scan candidates. Existing nonresident-metadata size
limits did not bound this allocation.

Impact is application availability when a consumer selects an untrusted volume
or partition image and starts default planning or parallel scanning. This does
not establish arbitrary code execution or writable device access. The finding
was traced independently in source. Regression reproduction used small synthetic
geometry; no enormous allocation or malicious live-device input was needed.

### Resolution and evidence

Both public planning methods now return `MftResult<Vec<RawMftWorkChunk>>`.
The shared planner validates the candidate count before bitmap filtering or
allocation, and reserves fallibly. Each plan is limited to 64 MiB of chunk
descriptors (4,194,304 chunks). This is a plan bound, not a total scan-memory
limit. Explicit selections apply the same bound after clipping empty ranges;
ordering, overlaps, and raw extent-capacity semantics remain unchanged.
Automatic rich, lean and cached-enrichment scans propagate planning failures.
The lean reader also avoids reserving a result vector from an untrusted record
count before reading.

Five new tests cover rejection before either bitmap-policy branch, exact limit
arithmetic without allocating a maximum-size plan, the final boundary near
`u64::MAX`, all automatic parallel projections, and a huge explicit lean chunk
that reaches an ordinary read error instead of a range-sized allocation.
Temporarily removing the allocation guard made the pre-filter regression fail
immediately at its sentinel assertion; restoring the guard made it pass.
Small selected ranges still parse successfully, and existing ordering,
overlap, extension and serial/parallel consistency tests pass. An independent
review found no concrete surviving bypass or regression in the changed paths.

The optional Codex Security scan stopped during discovery because its worker
hit a service cybersecurity access restriction. It was marked failed, not
complete. The targeted fix above is verified, but this review does not claim
a completed whole-repository security scan, fuzzing campaign, or security
clearance.

## Fixture provenance and remaining release checks

### Retained fixture privacy caveat

All five LFS payloads match their documented hashes. The sparse-volume verifier
passed four ranges, 1,280 record slots and 19 known records. Existing fixture
bytes and the previously accepted Windows installation-media provenance remain
unchanged.

The upstream MFT is an unchanged, publicly hosted sample with a pinned commit
and upstream MIT notice. Its bytes include profile/browser-cache metadata and
confidential-looking filenames; upstream licensing does not establish that the
sample is anonymized or establish consent for embedded contents. Its
[fixture notice](../../tests/fixtures/omerbenamram-mft/README.md#privacy) now makes
this limitation explicit and discourages reproducing potentially sensitive
names or contents. Retaining existing bytes preserves comparisons; it does not
resolve third-party rights or privacy uncertainty. New regressions use synthetic
data. No fixture bytes or Git history were changed.

### Restore native CI evidence

GitHub was private when checked. The latest baseline run,
[35679995242](https://github.com/wangfu91/ntfs-mft/actions/runs/35679995242), failed
before any steps ran because GitHub reported an account payment/spending-limit
block. Linux and macOS jobs were cancelled. A maintainer must resolve that
account issue or otherwise obtain passing native CI before release.

Private vulnerability reporting could not be confirmed: its endpoint returned
404. `SECURITY.md` provides the existing email fallback. Crates.io search found
no matching crate at review time; neither that result nor a dry run reserves
the name or establishes publishing-account access.

## Dependency and privacy review

Cargo Audit 0.22.2 checked RustSec revision
`ef6173cbc5c50ec8166f9a5b28f07834144373ee` (database updated October 3).
The final 128-dependency workspace lockfile had no vulnerability-class findings
and one informational unsoundness warning:
[RUSTSEC-2026-0253](https://rustsec.org/advisories/RUSTSEC-2026-0253.html) in
`lru` 0.16.4, used only by the pinned independent parser's test harness.
That parser uses `LruCache<u64, PathBuf>`; the advisory requires a potentially
panicking key destructor, which `u64` does not have. No reachable trigger was
identified, and the independent parser revision was preserved.

The generated package's 90-dependency lockfile excludes that parser and `lru`.
Its advisory check reported no vulnerabilities or informational warnings.
Runtime dependencies use registry sources and declare permissive license
options; this inventory does not replace review of fixture rights.

At the initial review baseline, targeted credential-pattern checks covered 90
reachable commits, 841 historical Git blobs, all five distinct historical/current
LFS payloads, 11 issue/PR bodies and 14 PR review comments. No recognized token,
private-key or credential-URL markers were found. This is a bounded pattern
check, not proof that all private data is absent. Deleted `docs/MFT.png` remains
in Git history; deleting its current-tree copy did not purge historical content.
No history rewrite was made.

## Local validation

| Check | Result |
| --- | --- |
| `cargo fmt --all -- --check` | Passed |
| `cargo test --workspace --locked` | 214 root unit tests, 28 root integration tests, 12 doctests, 11 differential comparisons and two Windows API decoder tests passed |
| `cargo test --workspace --release --locked` | Same test inventory passed with optimization |
| `cargo clippy --workspace --locked --all-targets -- -D warnings` | Passed, including both harnesses |
| Package-specific rustdoc, warnings denied | Passed |
| `cargo check --locked --lib --target x86_64-unknown-linux-gnu` | Passed; cross-compilation only |
| Rust snippets in Markdown | Six examples passed |
| Local Markdown links | 67 file targets and four heading fragments checked across 28 files |
| `cargo package --locked` | Passed from a clean committed checkout |
| `cargo publish --dry-run --locked` | Passed; upload explicitly aborted by dry-run mode |
| Tests from the extracted package | 214 unit, 28 integration and 12 doc tests passed |
| Fixture verification | All five LFS hashes matched; independent sparse verifier passed |

The differential harness also recompiles the 214 library unit tests, reporting
225 passes in that binary. Ordinary workspace runs ignored five live-device
tests; all five were then selected and passed separately as described below.
The first optimized run exposed two existing privilege-test failures; the final
run above includes their repair, rather than skipping those tests.

The archive inspected at `654d945` contained 120 files, about 1.4 MiB compressed.
It contained exact fixture bytes and required notices, no LFS pointers, and no
CI files, unpublished harnesses or build artifacts. The normalized runtime
manifest had no path/Git dependencies. Tests were run against its extracted
manifest with a separate target directory. Later report-only documentation
changes do not alter the tested library.

### Physical-device validation on this PC

Final binaries were run in an independent Administrator process via a temporary
scheduled task, with C: verified as NTFS and D: as ReFS. Each selected test ran
with `--ignored --nocapture --test-threads=1` and exited zero, with no skips:

- `volume_accessors`: volume geometry and NTFS accessors.
- `mft_byte_reader`: independent read handles and iteration; observed a
  4,744,282,112-byte MFT, 1,024-byte records and 28 valid records in the first
  32 slots.
- `raw_mft_parallel_chunk_consistency`: serial/parallel selected-chunk parity.
- `raw_mft_mft_consistency`: 500/500 Windows API records matched, zero transient
  misses.
- `refs_unsupported`: the ReFS volume was rejected as unsupported.

The final run completed on October 5 at 03:24 UTC. The temporary scheduled task
was removed and its absence independently checked. Local command logs, binary
hashes and structured results are under ignored `target/release-review/`;
these device results are separate from fixture and CI evidence.

No Linux/macOS native execution, remote docs.rs build, or fresh crates.io
consumer test ran in this review. No performance claim is made. Complete the
outstanding decisions and checks in
[the release checklist](../releasing.md) before publishing.
