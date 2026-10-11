# Contributing

Bug reports, documentation fixes, tests, and focused pull requests are welcome.
For a substantial API change, open an issue with the use case first.

Follow the [Code of Conduct](CODE_OF_CONDUCT.md). Report vulnerabilities privately
as described in [SECURITY.md](SECURITY.md).

## Setup

Use the latest stable Rust, a platform linker, Python 3, and [Git LFS](https://git-lfs.com/).
Windows builds require the MSVC toolchain and Visual Studio C++ build tools.
No minimum supported Rust version is declared.

```sh
git lfs install
git clone https://github.com/wangfu91/ntfs-mft.git
cd ntfs-mft
git lfs pull
python tests/fixtures/sparse-volume-fixture/verify.py
```

Use `python3` if needed. Tests and packaging require full fixture bytes; files
beginning with `version https://git-lfs.github.com/spec/v1` are LFS pointers.

## Validation

Run from the repository root. The workspace shares the tracked `Cargo.lock`;
published consumers resolve their own dependency graph.

```sh
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --locked --all-targets -- -D warnings
cargo doc --package ntfs-mft --locked --no-deps
cargo package --locked
cargo publish --dry-run --locked
```

Set `RUSTDOCFLAGS=-D warnings` for rustdoc. Document only `ntfs-mft`: the
differential harness compiles the same library source and would collide with
its documentation output. Packaging requires a clean checkout.

| Check | Scope |
| --- | --- |
| Root tests | Parser, metadata, synthetic I/O, fixtures, and public APIs |
| [Differential harness](differential-tests/README.md) | Comparison with a pinned independent parser |
| [Windows parity harness](integration-tests/README.md) | Portable decoders and opt-in Windows API comparisons |
| Native CI | Build, tests, Clippy, rustdoc, and packaging on Windows, Linux, and macOS |
| Live-device tests | The configured device, OS, permissions, and launch context |
| Package and publish dry run | Archive construction and verification build; no upload or remote docs.rs build |

Ordinary runs and CI ignore device tests. Record the revision, platform,
commands, results, and limitations; keep fixture, CI, and live-device evidence
separate. An older run does not establish current release readiness.

### Live-device tests

Use only devices you are authorized to inspect. Selected tests fail on missing
configuration, failed opens, unexpected filesystems, scan errors, or insufficient
comparison data. Ignored tests provide no device evidence.

From an independent Administrator PowerShell on Windows:

```powershell
$env:NTFS_MFT_TEST_DRIVE = 'C'
cargo test --locked --test mft_byte_reader --test volume_accessors --test raw_mft_parallel_chunk_consistency -- --ignored --nocapture
cargo test --manifest-path integration-tests/Cargo.toml --locked --test raw_mft_mft_consistency -- --ignored --nocapture
```

For the separate ReFS rejection test, set `NTFS_MFT_REFS_TEST_DRIVE` to an actual
ReFS volume and run `cargo test --locked --test refs_unsupported -- --ignored --nocapture`.
There are no default test drive letters. See [Windows access](docs/windows-raw-volume-access.md)
for launch restrictions.

On Linux/macOS, set `NTFS_MFT_TEST_VOLUME` to an NTFS mount root or device.
Build with `--no-run`, then execute the exact test binary with `--ignored --nocapture`
in a context allowed to read the device. See [macOS access](docs/macos-raw-volume-access.md).

## Benchmarks and profiling

The `mft_stream` benchmark and profiling executables share an offline workload:
the pinned fixture in memory or through a warm file, with extension resolution
disabled/enabled. Each scan checks the expected success/error inventory.

```sh
cargo bench --locked --bench mft_stream -- --save-baseline before
cargo run --locked --profile bench --example mft_stream_profile -- memory records 100
cargo run --locked --profile bench --example mft_stream_profile -- file extensions 100
cargo run --locked --profile bench --example mft_stream_allocations_profile
```

Preserve baseline and candidate executables built with the same harness,
toolchain, profile, and fixture. [The comparison script](tools/profiling/compare_mft_stream.ps1)
accepts `-Baseline`, `-Candidate`, `-Rounds`, and `-Iterations`; it alternates run
order and rejects mismatched result fingerprints. Keep the fixture at the path
used by both executables.

For live serial and parallel measurements, use [`raw_mft`](benches/raw_mft.rs)
and [`raw_mft_ingest`](benches/raw_mft_ingest.rs). On Windows, select the NTFS
drive with `NTFS_MFT_TEST_DRIVE`; on Linux/macOS, use `NTFS_MFT_TEST_VOLUME`.
The matching profile targets are `raw_mft_serial_read_profile` and
`raw_mft_parallel_ingest_profile`. Run with raw-device permissions and inspect
the output: a setup skip is not a measurement.

Keep volume state, selection, metadata options, worker/chunk settings, and
buffers consistent. Run timings without concurrent builds, benchmarks, or
profiling. Report repeated measurements and the platform/build configuration.
Allocation requests are separate from timings, retained heap, and working set.

## Changes and pull requests

Explain the problem, resulting behavior, and validation. Preserve read-only I/O,
narrow unsafe boundaries, and explicit errors and unknown metadata. Add focused
coverage and update API documentation and the changelog for behavior changes.
Keep unrelated refactoring out of a fix.

Maintainers preparing a release should follow the [release checklist](docs/releasing.md).

## Fixture privacy and provenance

Do not attach personal disk images, MFT captures, journals, or unreviewed exports
to public issues or PRs. An MFT can expose names, timestamps, identifiers,
deleted-record slack, and complete resident file contents.

Prefer minimal synthetic data. Any new binary fixture needs its source,
redistribution rights, privacy review, hash, extraction/generation steps,
expected results, and an independent verifier. Use Git LFS for large fixtures.
A changed fixture needs new provenance and expectations.

Contributions use the repository's [MIT license](LICENSE) unless stated otherwise.
Preserve third-party notices and submit only material you have the right to contribute.
