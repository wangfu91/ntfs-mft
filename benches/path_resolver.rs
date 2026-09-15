#[cfg(windows)]
#[path = "../support/benchmark_volume.rs"]
mod benchmark_volume;

#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
mod windows_bench {
    // Benchmark raw snapshot path construction and resolution (Windows).

    use criterion::{Criterion, criterion_group, criterion_main};
    use ntfs_mft::{
        raw_mft::{RawMft, RawMftEntry},
        volume::Volume,
    };

    /// Number of random entries to collect and resolve.
    const NUM_TEST_ENTRIES: usize = 1000;

    /// Collect a list of raw `$MFT` entries up to NUM_TEST_ENTRIES.
    fn collect_raw_test_entries(volume: &Volume) -> Vec<RawMftEntry> {
        let mft = match RawMft::new(volume) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("skipping: {e}");
                return Vec::new();
            }
        };

        let mut entries = Vec::new();
        if let Ok(it) = mft.scan_entries() {
            for entry in it.flatten() {
                entries.push(entry);
                if entries.len() >= NUM_TEST_ENTRIES {
                    break;
                }
            }
        }
        entries
    }

    /// Resolve paths using the RawMft-optimized resolver (one-time full $MFT scan).
    fn resolver_raw_mft_optimized(c: &mut Criterion) {
        let volume =
            match crate::benchmark_volume::open_volume(crate::benchmark_volume::pick_drive()) {
                Ok(volume) => volume,
                Err(error) => {
                    eprintln!("skipping bench: {error}");
                    return;
                }
            };

        let mft = match RawMft::new(&volume) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("skipping: {e}");
                return;
            }
        };

        let entries = collect_raw_test_entries(&volume);

        if entries.is_empty() {
            eprintln!("skipping: no entries collected");
            return;
        }

        c.bench_function("resolver_raw_mft_optimized", |b| {
            b.iter(|| {
                let resolver = match mft.path_resolver() {
                    Ok(r) => r,
                    Err(e) => {
                        eprintln!("skipping: {e}");
                        return 0u64;
                    }
                };

                let mut count = 0u64;
                for entry in &entries {
                    let _ = resolver.resolve_path(entry);
                    count += 1;
                }

                count
            })
        });
    }

    criterion_group!(path_resolver_benches, resolver_raw_mft_optimized);
    criterion_main!(path_resolver_benches);
    pub fn run() {
        main();
    }
}
#[cfg(windows)]
fn main() {
    windows_bench::run();
}
