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

Portable record-decoder tests run on every supported host. The live Windows
parity test is ignored by default. In an independent Administrator PowerShell:

```powershell
$env:NTFS_MFT_TEST_DRIVE = 'C'
cargo test --manifest-path integration-tests/Cargo.toml --locked --test raw_mft_mft_consistency -- --ignored --nocapture
```

The source must be explicitly configured and NTFS. Setup, enumeration, parsing,
and I/O failures fail the test. It compares up to 500 named V2 records, requires
at least 200 comparisons (or the full sample if smaller), and tolerates up to 5%
field disagreement due to concurrent filesystem changes. It does not silently
pass an empty comparison or discard enumeration errors.
