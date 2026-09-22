//! Logical MFT parsing from memory and a warm file; no device access is required.
use std::time::Duration;

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
#[path = "../support/mft_stream_workload.rs"]
mod workload;

fn stream_benchmarks(c: &mut Criterion) {
    let bytes = workload::FIXTURE;
    assert!(bytes.starts_with(b"FILE"), "hydrate the Git LFS fixture");
    let mut group = c.benchmark_group("mft_stream");
    group.throughput(Throughput::Elements(bytes.len() as u64 / 1024));
    for (name, source, resolve) in [
        ("records", workload::Source::Memory, false),
        ("extensions", workload::Source::Memory, true),
        ("file_records", workload::Source::File, false),
        ("file_extensions", workload::Source::File, true),
    ] {
        workload::check(&workload::run(source, resolve).unwrap_or_else(|error| fail(error)));
        group.bench_function(name, |b| {
            b.iter(|| {
                workload::check(
                    &workload::run(source, resolve).unwrap_or_else(|error| fail(error)),
                );
            });
        });
    }
    group.finish();
}

fn fail(error: ntfs_mft::MftError) -> ! {
    eprintln!("MFT stream benchmark setup failed: {error}");
    std::process::exit(1)
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .sample_size(30)
        .warm_up_time(Duration::from_secs(2))
        .measurement_time(Duration::from_secs(10));
    targets = stream_benchmarks
}
criterion_main!(benches);
