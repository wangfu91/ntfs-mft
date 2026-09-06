# Agent guide

This crate owns raw NTFS MFT reading on Windows/Linux. Preserve read-only I/O,
strong domain types and item-level errors. Use MftError/MftResult. Keep the
Windows FFI boundaries narrow. Run cargo test, Clippy with warnings denied, and
cargo package. Device tests must skip unsuitable environments. Use Criterion
for performance claims. See README.md and docs/EXTRACTION.md for the code scope.
