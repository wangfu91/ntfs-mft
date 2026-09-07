#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
mod windows_bench {
    // Benchmark raw snapshot path construction and resolution (Windows).
    use std::env;

    use criterion::{Criterion, criterion_group, criterion_main};
    use ntfs_mft::{
        errors::MftError,
        raw_mft::{RawMft, RawMftEntry},
        volume::Volume,
    };

    /// Number of random entries to collect and resolve.
    const NUM_TEST_ENTRIES: usize = 1000;

    /// Read the drive letter to benchmark from `NTFS_MFT_TEST_DRIVE`.
    fn pick_drive() -> char {
        env::var("NTFS_MFT_TEST_DRIVE")
            .ok()
            .and_then(|s| s.chars().next())
            .map(|c| c.to_ascii_uppercase())
            .unwrap_or('C')
    }

    /// Open the benchmark target volume or skip when the environment is unsuitable.
    fn open_volume() -> Option<Volume> {
        match Volume::from_drive_letter(pick_drive()) {
            Ok(v) => Some(v),
            Err(MftError::NotElevated) => {
                eprintln!("skipping bench: requires admin privileges");
                None
            }
            Err(e) => {
                eprintln!("skipping bench: {e}");
                None
            }
        }
    }

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
        if let Ok(it) = mft.try_iter() {
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
        let Some(volume) = open_volume() else { return };

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
