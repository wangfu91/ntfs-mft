# Windows API parity tests

Run from an independent `ntfs-mft` checkout:

```text
cargo test --manifest-path integration-tests/Cargo.toml --locked
```

The local test support enumerates records through `FSCTL_ENUM_USN_DATA` and
compares file references, parent references, names, and directory flags with the
raw parser. The strict parity test requests V2 records for standard NTFS IDs.
No sibling repository is required. The test helpers are not included in the
published library.

Live Windows tests require an elevated process and an NTFS volume
(`NTFS_MFT_TEST_DRIVE`, default `C`); unsuitable environments skip. Portable
record-decoder tests run on every supported host.
