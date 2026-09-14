# Captured NTFS volume fixture

I extracted this fixture read-only from my Windows installation USB, which
contains no personal files. The bytes are unchanged, including resident file
contents and record slack. The library's MIT license does not grant additional
rights to Microsoft-origin material.

| File | Contents | Bytes |
| --- | --- | ---: |
| `boot.bin` | NTFS boot sector | 512 |
| `mft.bin` | Complete logical `$MFT` | 1,310,720 |
| `mft-bitmap-0.bin` | First `$MFT::$BITMAP` fragment | 4,096 |
| `mft-bitmap-1.bin` | Remaining logical bitmap bytes | 8 |

The bitmap belongs to the MFT; it is not the volume allocation file `$Bitmap`.
Only logical stream bytes are included, without allocation padding.
Nonresident installer payloads, the backup boot sector, and MFT mirror contents
are excluded.

[`manifest.json`](manifest.json) contains partition-relative offsets, geometry,
runlists, SHA-256 hashes, and record counts. [`expected.json`](expected.json)
contains independently decoded metadata for 19 known records.

Tests cover bootstrapping, both bitmap fragments, 1,116 active records, serial
scanning, both parallel schedulers, and independently owned MFT byte streams.
Uncaptured reads fail. These offline tests require no device or elevated
privileges on Windows, Linux, or macOS; device access still needs live testing.
I kept the [upstream fixture](../omerbenamram-mft/README.md) for additional
coverage of unused records, extension records, and streams.

From the repository root, fetch the Git LFS files and verify:

```sh
git lfs pull
python3 tests/fixtures/captured-volume/verify.py
cargo test --lib raw_mft::tests::captured_volume
```
