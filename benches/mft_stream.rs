//! In-memory logical MFT parsing; no device access or elevation is required.
use std::{hint::black_box, io::Cursor, time::Duration};

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use ntfs_mft::{MftStream, MftStreamOptions};

fn stream_benchmarks(c: &mut Criterion) {
    let bytes = include_bytes!("../tests/fixtures/omerbenamram-mft/MFT");
    assert!(bytes.starts_with(b"FILE"), "hydrate the Git LFS fixture");
    let mut group = c.benchmark_group("mft_stream");
    group.throughput(Throughput::Elements(bytes.len() as u64 / 1024));
    for resolve in [false, true] {
        let options = MftStreamOptions::new(1024)
            .unwrap_or_else(|error| fail(error))
            .resolve_extensions(resolve);
        group.bench_function(if resolve { "extensions" } else { "records" }, |b| {
            b.iter(|| {
                let mut stream = MftStream::from_reader(Cursor::new(bytes.as_slice()), options)
                    .unwrap_or_else(|error| fail(error));
                for record in stream.iter_records() {
                    let _ = black_box(record);
                }
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
