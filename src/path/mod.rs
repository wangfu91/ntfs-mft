//! Internal snapshot tree and optional Windows live path fallback.
mod in_memory_tree;
pub use in_memory_tree::{MftPathError, MftPathIndex, MftPathQuality, MftPathResolution};
#[cfg(windows)]
pub(crate) mod resolve;

#[cfg(test)]
mod tests;
