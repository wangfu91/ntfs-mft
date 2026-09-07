# Serial MFT performance

Historical performance findings: the later public-API migration replaced native
name decoding with `NtfsName` UTF-16 storage. The name-decoding implementation
and measurements below describe the earlier representation.

These measurements describe the tested workloads, not a throughput guarantee.

| Workload | Baseline median | Optimized median |
| --- | ---: | ---: |
| Windows C:, 300,000 warm records | 888 ms | 797 ms (~10% faster) |
| Linux, read-only NTFS mount, 2,115,174 allocated records | 1.090 s | 1.023 s (6.18% faster) |

The Linux volume had 3,340,288 addressable slots; record counts were unchanged
and parsing reported no errors. A separate Windows full-volume baseline took
8.9–9.2 seconds for about 2.06 million records; it was not the 300,000-record
optimization comparison.

## Retained optimization

Filename accessors borrow UTF-16LE bytes instead of allocating an intermediate
`Vec<u16>`. `osstring_from_utf16le` constructs ASCII names directly. Windows
retains `OsString::from_wide` for non-ASCII input to preserve lone surrogates;
the Unix path streams UTF-16 decoding without the intermediate vector.
Odd-length input is rejected.

The Windows profile attributed about 30% of CPU time to filename materialization
and selection, and about 37% to attribute-list enrichment. These are inclusive
profile groups; overlapping subfunctions must not be added together. Linux name
decoding accounted for about 15% of sampled cycles.

Reducing `FileNameSelector` shrink-to-fit work and using `Box<OsStr>` did not
measurably improve performance, so I reverted those experiments. Extension
lookups remain costly because they perform random reads and parse attributes
that the base entry may not need. A lean extension parser or cache needs its
own workload measurements before adoption.

## Reproduction

Use `benches/raw_mft.rs` for Criterion measurements and
`examples/raw_mft_serial_read_profile.rs` for matching profiles. The Windows
comparison used Criterion 0.8.2 with 20 samples, 3 seconds of warm-up, and
30 seconds of measurement. Profiling used cargo-flamegraph with the
blondie/ETW backend in an administrator process. Linux profiling used `perf`;
select its source with `NTFS_MFT_TEST_VOLUME=<mount-or-device>`.

Re-profile after changes to name/link ownership or enrichment. Separate timing
runs from tracing, and control background I/O before attributing a gain to parsing.
