# Contributing

Bug reports, documentation fixes, tests, and focused pull requests are welcome.
For a substantial API change, open an issue describing the use case first so
maintainers and contributors can agree on scope.

Follow the [Code of Conduct](CODE_OF_CONDUCT.md). Report security problems
privately as described in [SECURITY.md](SECURITY.md).

## Setup

Use the latest stable Rust toolchain, a platform linker/toolchain, Python 3, and
[Git LFS](https://git-lfs.com/). Windows builds use the MSVC toolchain and Visual
Studio C++ build tools. CI covers Windows, Linux, and macOS.

```sh
git lfs install
git clone https://github.com/wangfu91/ntfs-mft.git
cd ntfs-mft
git lfs pull
python tests/fixtures/sparse-volume-fixture/verify.py
```

Use `python3` if that is your platform's Python command. The fixture files must
be hydrated before tests or packaging; a small text file beginning with
`version https://git-lfs.github.com/spec/v1` is a pointer, not a fixture.

## Validation

The workspace tracks its root `Cargo.lock` so development and CI checks use the
same dependency graph. Published library consumers resolve their own graph.

Run from the repository root:

```sh
cargo fmt --all -- --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo doc --locked --no-deps
cargo package --locked
cargo publish --dry-run --locked
```

For parser changes, also run the pinned comparison harness:

```sh
cargo test --manifest-path differential-tests/Cargo.toml --locked --lib differential_tests
cargo clippy --manifest-path differential-tests/Cargo.toml --locked --all-targets -- -D warnings
```

For Windows API or live-volume behavior, see the
[parity harness](integration-tests/README.md) and [validation guide](docs/VALIDATION.md).
Live-device tests are marked `#[ignore]`; ordinary test runs and CI execute
portable/fixture checks and report device checks as ignored. Running them with
`--ignored` requires explicit volume configuration and appropriate permissions.
Missing configuration, failed opens, unexpected filesystems, scan errors, and
insufficient comparison data fail the selected test; they are not success skips.
Use only devices you are authorized to inspect. Distinguish fixture, live-device,
and CI evidence when reporting results.

From an independent Administrator PowerShell on Windows:

```powershell
$env:NTFS_MFT_TEST_DRIVE = 'C'
cargo test --test mft_byte_reader --test volume_accessors --test raw_mft_parallel_chunk_consistency -- --ignored --nocapture
cargo test --manifest-path integration-tests/Cargo.toml --locked --test raw_mft_mft_consistency -- --ignored --nocapture
```

The separate ReFS check requires `NTFS_MFT_REFS_TEST_DRIVE` to name an actual
ReFS volume; select it explicitly with
`cargo test --test refs_unsupported -- --ignored --nocapture`.
There are no default test drive letters.

On Linux/macOS, set `NTFS_MFT_TEST_VOLUME` to the NTFS mount root or `/dev/...`
device. Build tests with `--no-run`, then run the exact executable printed by
Cargo with `--ignored --nocapture` in a context allowed to read that device.
See [macOS instructions](docs/macos-raw-volume-access.md) for an example.

The project does not declare or test a minimum supported Rust version. CI uses
the latest stable toolchain on each supported desktop OS.

Maintainers preparing a release should follow the
[GitHub and crates.io release checklist](docs/releasing.md).

## Changes and pull requests

- Explain the problem, resulting behavior, and validation evidence.
- Preserve read-only volume access, narrow unsafe/FFI boundaries, and explicit
  errors. Preserve unknown or partial metadata instead of inventing valid values.
- Add focused regression coverage for behavior changes. Document public API
  changes and update the changelog when appropriate.
- Keep unrelated refactoring out of a fix. For performance claims, include the
  workload, environment, and reproducible measurements.

## Offline performance measurements

`mft_stream` measures the licensed fixture in memory and through a warm file,
with extension resolution both disabled and enabled. It checks the known
success/error inventory on every iteration. Its profiling executables share
the same workload and require no raw-volume access:

```sh
cargo bench --locked --bench mft_stream -- --save-baseline before
cargo run --locked --profile bench --example mft_stream_profile -- memory records 100
cargo run --locked --profile bench --example mft_stream_profile -- file extensions 100
cargo run --locked --profile bench --example mft_stream_allocations_profile
```

Keep baseline and candidate executables built with the same harness, toolchain,
profile and fixture. `tools/profiling/compare_mft_stream.ps1` alternates their
run order and rejects mismatched result fingerprints. Use allocation counts
separately from timings; cumulative requested bytes are not retained heap or
working set. See the [performance audit](docs/audits/2026-09-21-performance.md)
for reproduction commands and measured scope.

## Fixture privacy and provenance

Do not attach personal disk images, raw MFT captures, USN journals, or unreviewed
scan exports to public issues or PRs. Even an MFT-only capture can expose names,
timestamps, identifiers, deleted-record slack, and complete resident file contents.

Prefer a minimal synthetic reproducer. Before proposing a binary fixture,
document its source, redistribution license or permission, privacy review,
hash, extraction or generation steps, and expected results. Put large fixtures
in Git LFS and provide an independent verifier. Do not silently sanitize an
existing fixture: a changed artifact needs new provenance and expectations.

Unless explicitly stated otherwise, contributions are provided under the
repository's [MIT license](LICENSE). Preserve required third-party notices and
only submit material you have the right to contribute.
