# Performance audit — 2026-09-21

Baseline: `c80b050cffdc15a2054933646394c5972e611fe2`.
The performance harness was added before production changes and is shared by
both compared versions. This audit uses existing licensed fixtures and
synthetic test records, with no personal volume captures.

## Measurement protocol

- Windows 11 Pro 10.0.26200; AMD Ryzen 9 5950X, 16 cores / 32 logical processors;
  Rust/Cargo 1.98.1, x86_64 MSVC, default features and system allocator.
- Cargo bench profile: optimized, debug symbols, thin LTO. No RUSTFLAGS,
  RUSTC_WRAPPER, CARGO_INCREMENTAL or NTFS_MFT environment overrides.
- One parser thread; 1,024-byte records, all 13,088 fixture slots; extensions
  disabled/enabled. No raw volume handles, worker/chunk settings or elevation
  needed by the measured workload.
- Fixture: `tests/fixtures/omerbenamram-mft/MFT`, SHA-256
  `c78f4968345b70783fbf6573d86c4bf300295ae26ddde4033dd1037b802e13c1`.
- In-memory `Cursor` and warm-cache `MftStream::from_path` with its unchanged
  8 KiB `BufReader`. File measurements include open/close and construction;
  they are not cold-storage or live-volume throughput measurements.
- Criterion: 30 samples, 2-second warmup and 10-second requested measurement
  per workload, saved baseline `audit-matched-before`. Only isolated runs are
  accepted; preliminary runs overlapping other workload probes were discarded.
- Preserve a baseline profiling executable before runtime changes. Alternate
  baseline/candidate process order in repeated paired batches; measure inside
  each executable, excluding process startup, fingerprinting and three warmups.
- Every timed scan checks 13,068 successful records and 20 expected errors.
  Untimed fingerprints cover the complete Debug representation of every result,
  including lossless names, metadata and diagnostics: `aa46e4f416524fb4` without
  extensions, `6c0fce2240a5a77c` with extensions. Fingerprints are toolchain-local
  comparison oracles, not a public serialization format.
- Allocation counting is a separate instrumented probe using the same workload
  and `System` allocator; its timings are not used. Requested bytes include
  cumulative allocation/reallocation requests, not retained memory or RSS.

## Findings and results

### P1 — Sequential seeks discard file read-ahead (fixed)

`MftStream::load_into` sought to every record's absolute offset, even when the
source was already there. With `BufReader<File>`, those seeks discarded buffered
bytes and forced another file read for each 1 KiB slot.

Track the known source position and seek only for nonsequential access. Clear
the position before fallible I/O, restore it only after a complete read, and
retain it after a recoverable parse error. Extension reads and explicit random
access use the same tracked state. This adds one optional cursor field per
stream, with no per-record retained allocation or public API change.

The regression for 16 synthetic slots (one corrupt) failed on the baseline
with 18 seeks. It now performs only two constructor seeks and two 8 KiB reads,
while still returning 15 records and one error. A separate fault-injection test
verifies explicit-read recovery after either a read or seek error moves the
underlying cursor. Existing extension/cursor and terminal-I/O tests also pass.

### P2 — Validation materializes unused runlists (fixed)

`RecordInspection::observe` called `decode_runs(...).is_err()`, allocating and
growing a vector for each nonresident DATA attribute despite needing only its
validity. `validate_runs` now uses the existing checked visitor with a no-op
consumer. Decoding and validation retain the same format, truncation, signed
offset and overflow checks; bootstrap callers that need actual runs still
materialize them.

Separate allocation instrumentation over one complete in-memory scan measured:

| Extensions | Allocations before → after | Reallocations before → after | Cumulative requested bytes before → after |
| --- | --- | --- | --- |
| Disabled | 68,165 → 58,223 | 5,863 → 5,789 | 5,779,260 → 4,788,348 |
| Enabled | 68,204 → 58,258 | 5,868 → 5,794 | 5,789,202 → 4,797,906 |

This removes about 14.6% of allocation calls and 17.1% of requested bytes on
this fixture. These are cumulative requests, not live heap, working set or
retained entry size. The allocation probe uses the same `System` forwarding
and counting logic for both versions, separately from timing measurements.

### P3 — Benchmark coverage missed buffered-file behavior (fixed)

The original benchmark used only an in-memory `Cursor`, so it could not reveal
P1. It also discarded all results without checking the workload inventory.
The benchmark now covers both sources, checks success/error counts on every
scan, and shares its workload with profiling and allocation executables. The
paired runner compares full-result fingerprints outside the timed section.

### Matched results

Eight pairs per workload, 30 complete scans per process and three untimed
warmups, alternating baseline-first/candidate-first. Values below are medians
of process-batch time per scan. Change is the median of the eight paired
percentage changes, so it need not equal the ratio of the displayed medians.

| Workload | Before (ms) | After (ms) | Paired time change | Bootstrap 95% interval |
| --- | ---: | ---: | ---: | --- |
| Memory, records | 7.2821 | 6.7082 | -6.92% | -11.41% to -2.56% |
| Memory, extensions | 7.2084 | 6.7137 | -6.83% | -8.98% to -5.11% |
| Warm file, records | 41.1953 | 11.7132 | -71.44% | -71.72% to -71.38% |
| Warm file, extensions | 41.5809 | 12.0668 | -71.24% | -71.58% to -70.87% |

All 64 processes retained the baseline fingerprints and inventory. Intervals
use 10,000 bootstrap resamples of the eight paired changes, seed 20260921,
percentile indices 250 and 9749. Eight pairs characterize this run, not every
machine or workload. [Raw paired measurements](2026-09-21-performance-paired.csv)
retain the order, iteration counts, times and fingerprints.

Criterion also showed lower time in all four workloads; its estimated means
improved about 20% in memory and 72–74% for files. The lower in-memory gain in
the alternating runs demonstrates run-order/system drift, so the paired results
above are the primary claim. [Criterion estimates](2026-09-21-performance-criterion.json)
retain both versions' means, medians, slopes and confidence intervals. The
reported improvements combine P1/P2; no isolated time attribution is claimed.

## Reproduction

Start with the baseline revision plus **only** this audit's harness files and
Cargo example entries. Use the same fixture and working directory for both
versions. Before applying the runtime fixes:

```powershell
New-Item -ItemType Directory -Force target/performance-audit | Out-Null
cargo build --locked --profile bench --example mft_stream_profile
Copy-Item target/release/examples/mft_stream_profile.exe target/performance-audit/baseline.exe
cargo bench --locked --bench mft_stream -- --save-baseline audit-matched-before
cargo run --locked --profile bench --example mft_stream_allocations_profile
```

Apply the runtime fixes, keeping the harness unchanged, then run sequentially:

```powershell
cargo build --locked --profile bench --example mft_stream_profile
Copy-Item target/release/examples/mft_stream_profile.exe target/performance-audit/candidate.exe
cargo bench --locked --bench mft_stream -- --baseline audit-matched-before
./tools/profiling/compare_mft_stream.ps1 -Baseline target/performance-audit/baseline.exe -Candidate target/performance-audit/candidate.exe -Rounds 8 -Iterations 30
cargo run --locked --profile bench --example mft_stream_allocations_profile
```

Do not run compilation, other benchmarks or profiling concurrently with timing.
The fixture must remain at the path embedded in the profiling executable.

## Validation

- `cargo fmt --all -- --check`: passed.
- `cargo test --locked`: 208 unit tests, 28 integration tests and 12 doctests
  passed; four live-device tests remained ignored.
- `cargo clippy --locked --all-targets -- -D warnings`: passed, including the
  shared harness, both profiling examples and benchmarks.
- `cargo doc --locked --no-deps`: passed.
- Differential harness: all 11 tests passed; all-target Clippy passed.
- `cargo package --locked --allow-dirty --offline`: package creation and
  verification build passed using cached locked dependencies. The prior audit
  established the environment's online Windows TLS credential failure.
- `git diff --check`: passed.

## Scope and limits

Reviewed logical stream I/O, attribute inspection/runlist validation, name
materialization, raw buffered reads and extent mapping, bootstrap discovery,
attribute-list enrichment, chunk planning, ordinary ordered parallel execution,
cached enrichment, and benchmark/profiling helpers.

No further measured bottleneck was established in the raw scan paths. Ordered
parallel execution can retain completed chunks behind a slow earlier chunk,
and cached enrichment retains extension bytes/deferred bases by design. Name
clones support owned names/links; bootstrap prefix rebuilding handles staged
extent discovery. These remain workload-dependent profiling candidates, not
proven regressions, and their scheduling/representation policies were preserved.

No live-device, cold-cache, native Linux/macOS, raw-I/O scaling or retained-memory
measurement was performed. `cargo flamegraph` was unavailable; allocation
instrumentation, counted I/O and matched timings provide the evidence here,
not sampled CPU stacks. No CPU or storage saturation claim is made.
