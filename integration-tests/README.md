# Cross-repository parity tests

Keep `ntfs-mft` and `usn-journal-rs` as sibling checkouts. From `ntfs-mft`, run:

```text
cargo test --manifest-path integration-tests/Cargo.toml
```

Windows tests compare raw records with FSCTL enumeration. They require an elevated
process and an NTFS volume (`USN_TEST_DRIVE`, default `C`); unsuitable environments
skip. This package is not published and neither library depends on the other.
