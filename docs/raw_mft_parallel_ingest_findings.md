# Parallel MFT performance

On the tested Windows C: workload, dynamic scheduling performed best around
10–11 workers. The best measured configuration was 11 workers, 2,048-record
chunks, and 256 KiB/16 KiB main/attribute buffers (~2.35 seconds).
These results guide benchmark defaults, not every caller's scan configuration.

## Workload and method

The volume had 3,059,968 addressable records. Scanning from record 24 to the end
produced about 1,329 chunks at 2,048 records per chunk. Both planning and scanning
excluded unused records. The planner retained dense bands containing any used
record and omitted fully unused bands; splitting at every used-record run had
regressed performance.

Criterion and the profile example share `support/raw_mft_ingest_support.rs`.
The lean batch parser avoids building full entries for folded consumers.
Use 10 samples, 3 seconds of warm-up, and 10 seconds of measurement for the
sweeps below. Results from different sweeps are separate runs.

```powershell
$env:USN_RAW_MFT_BENCH_DRIVE='C'
$env:USN_RAW_MFT_BENCH_SCHEDULING='dynamic'
$env:USN_RAW_MFT_BENCH_CHUNK_RECORDS='2048'
$env:USN_RAW_MFT_BENCH_BUFFER_BYTES='262144'
$env:USN_RAW_MFT_BENCH_ATTR_BUFFER_BYTES='16384'
$env:USN_RAW_MFT_BENCH_WORKERS_LIST='1,2,4,6,8,10,11,12'
cargo bench --bench raw_mft_ingest -- --sample-size 10 --warm-up-time 3 --measurement-time 10
```

For scheduling sweeps, set `USN_RAW_MFT_BENCH_WORKERS=11` and
`USN_RAW_MFT_BENCH_SCHEDULING_LIST=dynamic,contiguous`, clearing the worker list.
`USN_RAW_MFT_BENCH_PRINT_SUMMARY=1` enables a summary;
`USN_RAW_MFT_BENCH_SUMMARY_RUNS` controls its repetitions. Linux and macOS select
sources with `USN_TEST_VOLUME=<mount-or-device>`.

## Windows sweeps

Chunk-size sweep medians (`workers=10`, `dynamic`,
default buffers at the time):

| Chunk records | Planned chunks | Median time |
| ------------- | -------------: | ----------: |
| 1,024         |          2,595 |     ~2.53 s |
| 2,048         |          1,329 |     ~2.49 s |
| 4,096         |            680 |     ~2.51 s |
| 8,192         |            351 |     ~2.56 s |

Observed medians from the follow-up worker and scheduling sweeps with
`chunk_records=2048`:

| Mode       | Workers | Median time |
| ---------- | ------: | ----------: |
| Dynamic    |       8 |     ~2.55 s |
| Dynamic    |      10 |     ~2.50 s |
| Dynamic    |      11 |     ~2.48 s |
| Dynamic    |      12 |     ~2.51 s |
| Contiguous |      11 |     ~3.67 s |

Observed medians from the main-buffer sweep with `chunk_records=2048`,
`workers=11`, `dynamic`, and `attr_buffer=16 KiB`:

| Main buffer | Median time |
| ----------- | ----------: |
| 64 KiB      |     ~2.51 s |
| 128 KiB     |     ~2.38 s |
| 256 KiB     |     ~2.35 s |
| 512 KiB     |     ~2.38 s |
| 1 MiB       |     ~2.41 s |
| 2 MiB       |     ~2.44 s |

Observed medians from the attribute-buffer sweep with `chunk_records=2048`,
`workers=11`, `dynamic`, and `main_buffer=256 KiB`:

| Attribute buffer | Median time |
| ---------------- | ----------: |
| 4 KiB            |     ~2.39 s |
| 8 KiB            |     ~2.35 s |
| 16 KiB           |     ~2.35 s |
| 32 KiB           |     ~2.41 s |
| 64 KiB           |     ~2.64 s |

The harness defaults to available parallelism capped at 10 workers, 2,048-record
chunks, and 256 KiB/16 KiB buffers. An 8 KiB attribute buffer tied 16 KiB;
larger buffers regressed on this workload.

## Rejected experiments

I reverted raw UTF-16 batch names and an ingest-specific fold that precomputed
visible parent references because both regressed. Batched dynamic scheduling
also failed confirmation: its initial two-chunk improvement did not repeat.
The following runs used 2,048-record chunks and 256 KiB/16 KiB buffers, initially
with 11 workers.

| Scheduling                | Median time |
| ------------------------- | ----------: |
| Dynamic                   |     ~2.43 s |
| Dynamic batched, 2 chunks |     ~2.38 s |
| Dynamic batched, 4 chunks |     ~2.43 s |
| Dynamic batched, 8 chunks |     ~2.51 s |
| Contiguous                |     ~3.36 s |

Confirmation runs did not retain the apparent two-chunk gain:

| Workers | Dynamic | Dynamic batched, 2 chunks |
| ------: | ------: | ------------------------: |
|      10 | ~2.34 s |                   ~2.36 s |
|      11 | ~2.35 s |                   ~2.36 s |

## I/O evidence

Matched ETW runs used 11 workers and the same chunk/buffer settings, changing
only scheduling. Profile times include tracing overhead and are separate from
Criterion medians.

| Scheduling | Profile elapsed |
| ---------- | --------------: |
| Dynamic    |        ~2.692 s |
| Contiguous |        ~4.410 s |

Quick `xperf -a diskio -detail` summaries for the profile process showed:

| Scheduling | Read count | Read MiB | Avg read I/O us | Avg read service us | Avg read QD start |
| ---------- | ---------: | -------: | --------------: | ------------------: | ----------------: |
| Dynamic    |     58,693 |  ~3228.0 |          ~186.7 |               ~38.0 |             ~5.08 |
| Contiguous |     58,546 |  ~3205.6 |          ~154.7 |               ~56.8 |             ~2.21 |

An earlier ETW run reached 93–99% disk utilization with substantial background
writes. CPU profiles also showed name cloning/decoding, link allocation,
attribute-list enrichment, and volume reads. Storage and background activity
therefore affect the result alongside parsing.

Read-offset jumps, ordered by request start across all workers:

| Scheduling | Exact sequential % | Jump <= 1 MiB % | Median abs jump |
| ---------- | -----------------: | --------------: | --------------: |
| Dynamic    |            `0.09%` |         `3.91%` |     ~`88.6 MiB` |
| Contiguous |            `0.92%` |         `7.63%` |    ~`885.9 MiB` |

These interleaved requests do not measure per-worker locality directly.
Contiguous scheduling had more adjacent reads but was slower overall. Higher
queue depth and shorter elapsed time under dynamic scheduling support the
inference that overlap and load balance mattered more than those locality gains.
They do not isolate the exact split among disk, kernel caching, CPU, and memory.

Each worker performs I/O and parsing with two independent readers. Equal record
ranges have unequal costs because allocation density, names, links, and extension
reads differ. Dynamic workers take more chunks as they finish; fixed contiguous
bands can leave a slow tail. More workers also add competing I/O and buffers,
which is consistent with the measured plateau.

To capture a matching profile, build the example first, set the same environment
variables, and run from an administrator terminal:

```powershell
cargo build --release --example raw_mft_parallel_ingest_profile
wpr -start GeneralProfile -start FileIO -start DiskIO -filemode
target\release\examples\raw_mft_parallel_ingest_profile.exe
wpr -stop raw_mft_parallel_ingest.etl
```

Repeat with the other scheduling mode and a different output filename. Use a
quiet disk and process-specific ETW/WPA statistics before changing scheduling.

## Linux measurement

On a read-only NTFS volume with 3,340,288 addressable records, 10-worker dynamic
ingest changed from 204.9 ms to 203.2 ms after the filename-decoder optimization.
The difference was within the configured noise threshold, so it does not establish
a parallel-ingest improvement.
