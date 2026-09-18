# Agent guide

## Scope

`ntfs-mft` provides read-only NTFS Master File Table parsing and scanning on
Windows, Linux, and macOS. It owns raw-volume access, `$MFT` bootstrap and extent
handling, record parsing, metadata extraction, and serial/parallel scanning.
Filesystem trees, full-path reconstruction, application scan policy, and USN
journal APIs belong to consumers or other crates.

Consult these files before changing behavior:

- `README.md` for the public feature set and limitations.
- `docs/api-guide.md` for API selection and error contracts.
- `docs/raw-mft-read.md` and `src/raw_mft/README.md` for the implementation map.
- `CONTRIBUTING.md` for complete validation and live-device instructions.

## Invariants

- Keep all volume and device access read-only. Do not add filesystem mutation or
  writable-handle behavior.
- Preserve raw evidence. Represent unknown, unavailable, or partially parsed
  metadata explicitly instead of inventing valid values.
- Keep recoverable record corruption as an item-level error where iteration can
  safely continue. I/O and structural failures that invalidate a scan must remain
  terminal.
- Use `MftError` and `MftResult` for crate operations. Preserve strong domain
  types and checked arithmetic at disk-format and offset boundaries.
- Keep platform-specific unsafe code and FFI narrow. Put shared parsing policy
  outside platform modules and gate OS-specific code with `#[cfg(...)]`.
- Treat exported types and behavior as a compatibility surface. Keep API changes
  focused, update public documentation, and record user-visible changes in
  `CHANGELOG.md`.
- The crate denies Clippy `unwrap_used`, `expect_used`, `panic`, `todo`,
  `unimplemented`, and `unreachable`; do not bypass those lints to handle normal
  errors.

## Repository layout

- `src/raw_mft/layout/`: NTFS on-disk structures, fixups, attributes, and data
  runs.
- `src/raw_mft/bootstrap/`: boot-sector and `$MFT` discovery.
- `src/raw_mft/serial/` and `src/raw_mft/parallel/`: scan engines and scheduling.
- `src/raw_mft/stream.rs`: logical extracted-MFT parsing.
- `src/volume/`: platform-specific read-only volume access.
- `tests/fixtures/`: licensed or synthetic binary fixtures and their provenance.
- `differential-tests/`: comparison against the pinned independent parser.
- `integration-tests/`: Windows API and live-volume parity checks.

## Validation

Run the smallest relevant tests while developing. Before handing off a code or
public-API change, run from the repository root:

```text
cargo fmt --all -- --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo doc --locked --no-deps
cargo package --locked
```

Parser changes also require the differential harness:

```text
cargo test --manifest-path differential-tests/Cargo.toml --locked --lib differential_tests
cargo clippy --manifest-path differential-tests/Cargo.toml --locked --all-targets -- -D warnings
```

Hydrate Git LFS before fixture tests or packaging. A small text file beginning
with the Git LFS version marker is a pointer, not a valid MFT fixture.

Live-device tests are ignored by default and require explicit configuration and
appropriate privileges. Once selected, failed opens, wrong filesystems, scan
errors, and insufficient comparison data are failures rather than successful
skips. Report fixture, CI, and live-device evidence separately; do not claim a
platform was validated from another platform's results.

## Fixtures and performance

Never add personal disk captures, MFTs, journals, or scan exports. Prefer small
synthetic fixtures. Any new binary fixture needs documented provenance,
redistribution rights, privacy review, a hash, expected results, and an
independent verifier.

Use Criterion or the matching profiling executables for performance work. State
the workload, platform, permissions, build profile, and repeated measurements;
do not make performance claims from a single informal run.
