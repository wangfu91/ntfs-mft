# Release review — 2026-10-05

Baseline: `a973e53ead27b99fe4e48956def510cffc91e580`.
Documentation and release-guide fixes: `16eebec` and `90bd44c`.
Host: Windows x86_64 MSVC, Rust/Cargo 1.98.1.

**Not ready to publish yet.** Chunk planning needs a resource bound, the upstream
fixture needs a maintainer privacy decision, and current native CI evidence is
unavailable. No repository visibility, crate publication, tag, push, release or
account setting was changed during this review.

## Changes made

- Shortened the README and compiled its examples along with the API and internal
  guides. Corrected offline-versus-device permissions, extension-I/O behavior,
  metadata completeness, and stale rustdoc links to the former default branch.
- Corrected macOS live-test instructions: explicitly selected tests fail when
  configuration is missing. Updated contribution templates to the current API.
- Separated historical validation from the current validation guide, preserving
  the historical evidence. Documented package-specific rustdoc, workspace
  validation, archive inspection and the limits of a publish dry run.
- Corrected the release checklist's implication that both fixtures carry license
  grants. The library and upstream fixture have MIT notices; the Windows fixture
  has provenance and an explicit Microsoft-content rights caveat.

## Outstanding release decisions

### Bound chunk planning before release

Static review identified a resource-exhaustion path in
`src/raw_mft/chunk_plan.rs` and `src/raw_mft/parallel/chunks.rs`.
Bootstrap validates arithmetic and the 48-bit record-number limit, but a small
input can claim a very large extent capacity without supplying those bytes.
Default planning uses that capacity and eagerly creates the complete chunk
vector before a scan read can fail at EOF. Missing allocation evidence keeps
the claimed ranges as scan candidates. Existing nonresident-metadata size
limits do not bound this allocation.

Impact is application availability when a consumer selects an untrusted volume
or partition image and starts default planning or parallel scanning. This does
not establish arbitrary code execution or writable device access. The finding
was traced independently in source; no enormous allocation or live-device
reproduction was attempted.

Proposed fix: bound the plan before allocation or record filtering, reserve
fallibly, and return `MftResult<Vec<RawMftWorkChunk>>` from both planning methods.
Automatic parallel scans must propagate the error; examples and callers must
handle it. Preserve explicit ranges, ordering, and raw extent-capacity semantics.
Regression checks should use small synthetic geometry and verify immediate
rejection without large allocation, plus ordinary and boundary-sized plans.
The change to the unpublished API is awaiting the maintainer's decision.

The optional Codex Security scan stopped during discovery because its worker
hit a service cybersecurity access restriction. It was marked failed, not
complete. The source finding above is retained, but this review does not claim
a completed whole-repository security scan, fuzzing campaign, or security
clearance.

### Resolve fixture privacy explicitly

All five LFS payloads match their documented hashes. The sparse-volume verifier
passed four ranges, 1,280 record slots and 19 known records. Existing fixture
bytes and the previously accepted Windows installation-media provenance remain
unchanged.

The upstream MFT is an unchanged, publicly hosted sample with a pinned commit
and upstream MIT notice. Its bytes include profile/browser-cache metadata and
confidential-looking filenames; upstream licensing does not establish that the
sample is anonymized. The maintainer was asked whether to retain it with an
explicit caveat or replace it with synthetic coverage before publication.

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
The 131-dependency workspace lockfile had no vulnerability-class findings and
one informational unsoundness warning:
[RUSTSEC-2026-0253](https://rustsec.org/advisories/RUSTSEC-2026-0253.html) in
`lru` 0.16.4, used only by the pinned independent parser's test harness.
That parser uses `LruCache<u64, PathBuf>`; the advisory requires a potentially
panicking key destructor, which `u64` does not have. No reachable trigger was
identified, and the independent parser revision was preserved.

The generated package's 93-dependency lockfile excludes that parser and `lru`.
Its advisory check reported no vulnerabilities or informational warnings.
Runtime dependencies use registry sources and declare permissive license
options; this inventory does not replace review of fixture rights.

Targeted credential-pattern checks covered 90 reachable commits, 841 historical
Git blobs, all five distinct historical/current LFS payloads, 11 issue/PR bodies
and 14 PR review comments. No recognized token, private-key or credential-URL
markers were found. This is a bounded pattern check, not proof that all private
data is absent. Deleted `docs/MFT.png` remains in Git history; deleting its
current-tree copy did not purge historical content. No history rewrite was made.

## Local validation

| Check | Result |
| --- | --- |
| `cargo fmt --all -- --check` | Passed |
| `cargo test --locked` | 208 unit tests, 28 integration tests and 12 doctests passed; four live-device tests ignored |
| Root Clippy, all targets, warnings denied | Passed |
| Package-specific rustdoc, warnings denied | Passed |
| Pinned differential tests and strict Clippy | 11 comparisons passed; Clippy passed |
| Windows API harness and strict Clippy | Two decoder tests passed; one live parity test ignored; Clippy passed |
| Rust snippets in Markdown | Six examples passed |
| `cargo package --locked` | Passed from a clean committed checkout |
| `cargo publish --dry-run --locked` | Passed; upload explicitly aborted by dry-run mode |
| Tests from the extracted package | Same 208 unit, 28 integration and 12 doc tests passed; four device tests ignored |

The archive inspected at `90bd44c` contained 119 files, about 1.4 MiB compressed.
It contained exact fixture bytes and required notices, no LFS pointers, and no
CI files, unpublished harnesses or build artifacts. The normalized runtime
manifest had no path/Git dependencies. Later report-only documentation changes
do not alter the tested library.

No physical-device tests, Linux/macOS native execution, remote docs.rs build,
or fresh crates.io consumer test ran in this review. No performance claim is
made. Complete the outstanding decisions and checks in
[the release checklist](../releasing.md) before publishing.
