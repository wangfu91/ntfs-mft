//! Parallel raw-MFT chunk scanning.

mod cached_enrich;
mod chunks;
mod executor;
mod scan;

pub(crate) use executor::ChunkScheduling;
pub use scan::{RawMftParallelScan, RawMftParallelScheduling};
