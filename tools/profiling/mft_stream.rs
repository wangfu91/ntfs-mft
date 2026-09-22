//! Exact Criterion fixture workload, with an untimed full-result fingerprint.
use std::{collections::hash_map::DefaultHasher, hash::Hasher, time::Instant};

#[path = "../../support/mft_stream_workload.rs"]
mod workload;

fn main() {
    if let Err(error) = run() {
        eprintln!("MFT stream profile failed: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let source = match args.next().as_deref() {
        Some("memory") => workload::Source::Memory,
        Some("file") => workload::Source::File,
        _ => {
            return Err(
                "usage: mft_stream_profile <memory|file> <records|extensions> <iterations>".into(),
            );
        }
    };
    let resolve = match args.next().as_deref() {
        Some("records") => false,
        Some("extensions") => true,
        _ => return Err("expected records or extensions".into()),
    };
    let iterations: std::num::NonZeroUsize = args.next().ok_or("missing iterations")?.parse()?;
    if args.next().is_some() {
        return Err("unexpected argument".into());
    }
    // Debug includes lossless names, all entry fields, metadata and diagnostics.
    // Fingerprinting is deliberately outside the measured workload.
    let mut fingerprint = DefaultHasher::new();
    let summary = workload::visit(source, resolve, |record| {
        fingerprint.write(format!("{record:?}\n").as_bytes());
    })?;
    workload::check(&summary);
    for _ in 0..3 {
        workload::check(&workload::run(source, resolve)?);
    }
    let start = Instant::now();
    for _ in 0..iterations.get() {
        workload::check(&workload::run(source, resolve)?);
    }
    let nanos = start.elapsed().as_nanos();
    println!(
        "source={source:?} extensions={resolve} iterations={iterations} ns={nanos} records={} errors={} fingerprint={:016x}",
        summary.records,
        summary.errors,
        fingerprint.finish()
    );
    Ok(())
}
