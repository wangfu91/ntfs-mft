//! Internal snapshot tree and optional Windows live path fallback.
mod in_memory_tree;
pub(crate) use in_memory_tree::InMemoryDirTree;
#[cfg(windows)]
pub(crate) mod resolve;

#[cfg(all(test, windows))]
mod tests;
