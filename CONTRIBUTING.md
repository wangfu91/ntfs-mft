# Contributing

Bug reports, documentation fixes, tests, and focused pull requests are welcome.
For a substantial API change, open an issue describing the use case first so
maintainers and contributors can agree on scope.

Follow the [Code of Conduct](CODE_OF_CONDUCT.md). Report security problems
privately as described in [SECURITY.md](SECURITY.md).

## Setup

Use Rust 1.88 or newer, a platform linker/toolchain, Python 3, and
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

Run from the repository root:

```sh
cargo fmt --all -- --check
cargo test --locked
cargo test --locked --all-features
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo doc --locked --all-features --no-deps
cargo package --locked
```

For parser changes, also run the pinned comparison harness:

```sh
cargo test --manifest-path differential-tests/Cargo.toml --locked --lib differential_tests
cargo clippy --manifest-path differential-tests/Cargo.toml --locked --all-targets --all-features -- -D warnings
```

For Windows API or live-volume behavior, see the
[parity harness](integration-tests/README.md) and [validation guide](docs/VALIDATION.md).
Tests may skip device access when privileges or a suitable volume are unavailable.
State which checks ran, which skipped, and whether results came from fixtures,
live devices, or CI. Do not treat a skipped device check as evidence of device
correctness. Use only devices you are authorized to inspect.

The minimum supported Rust version (MSRV) is 1.88. CI checks the library with all
features on Rust 1.88.0 on each supported desktop OS. To reproduce locally:

```sh
rustup toolchain install 1.88.0 --profile minimal
cargo +1.88.0 test --locked --lib --all-features
```

Call out any proposed MSRV increase in the PR. Keep the separate harness
dependency declarations in sync where their build scripts require it.

## Changes and pull requests

- Explain the problem, resulting behavior, and validation evidence.
- Preserve read-only volume access, narrow unsafe/FFI boundaries, and explicit
  errors. Preserve unknown or partial metadata instead of inventing valid values.
- Add focused regression coverage for behavior changes. Document public API
  changes and update the changelog when appropriate.
- Keep unrelated refactoring out of a fix. For performance claims, include the
  workload, environment, and reproducible measurements.

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
