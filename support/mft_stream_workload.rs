//! Shared fixture workload for Criterion and the offline profiling executable.
use std::{hint::black_box, io::Cursor};

use ntfs_mft::{MftResult, MftStream, MftStreamOptions};

pub const FIXTURE: &[u8] = include_bytes!("../tests/fixtures/omerbenamram-mft/MFT");
const FIXTURE_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/omerbenamram-mft/MFT"
);

#[derive(Clone, Copy, Debug)]
pub enum Source {
    Memory,
    File,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub records: usize,
    pub errors: usize,
}

/// Consume all results, including the fixture's known malformed slots.
pub fn run(source: Source, resolve: bool) -> MftResult<Summary> {
    visit(source, resolve, |_| {})
}

pub fn visit(
    source: Source,
    resolve: bool,
    mut observe: impl FnMut(&MftResult<ntfs_mft::MftRecord>),
) -> MftResult<Summary> {
    let options = MftStreamOptions::new(1024)?.resolve_extensions(resolve);
    let mut summary = Summary::default();
    let mut consume = |record: MftResult<ntfs_mft::MftRecord>| {
        observe(&record);
        if record.is_ok() {
            summary.records += 1;
        } else {
            summary.errors += 1;
        }
        let _ = black_box(record);
    };
    match source {
        Source::Memory => {
            MftStream::from_reader(Cursor::new(FIXTURE), options)?
                .iter_records()
                .for_each(&mut consume);
        }
        Source::File => {
            MftStream::from_path(FIXTURE_PATH, options)?
                .iter_records()
                .for_each(&mut consume);
        }
    }
    Ok(summary)
}

pub fn check(summary: &Summary) {
    assert_eq!(summary.records, 13_068, "fixture record inventory changed");
    assert_eq!(summary.errors, 20, "fixture error inventory changed");
}
