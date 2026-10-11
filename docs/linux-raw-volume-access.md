# Linux raw NTFS access

`Volume::from_mount_point("/media/user/windows")` resolves the exact mount root
through `/proc/self/mountinfo` and opens its backing device read-only. Symlinks
and trailing separators are canonicalized; child directories are rejected.
Accepted mount types are `ntfs`, `ntfs3`, and `fuseblk`; the mount source must
be an absolute device path. `fuseblk` alone does not prove NTFS:
`RawMft` validates the boot sector before scanning.

`Volume::from_device_path("/dev/sdXN")` also accepts an unmounted partition device
or a partition image file. Byte zero must be the NTFS boot sector, not a
whole-disk partition table. An extracted logical `$MFT` is a different input;
use [`MftStream`](offline-mft.md) for that input.

## Permissions and testing

Reading mounted files does not establish permission to read the backing device.
OS access errors are returned as `MftError::Io`; the library does not elevate
itself or change permissions. Partition image files need ordinary read permission.
All access is read-only, and live scans are not atomic filesystem snapshots.

Build as the ordinary user, then run in a context allowed to read the device:

```sh
cargo build --locked --example raw_mft_serial_read
sudo ./target/debug/examples/raw_mft_serial_read /media/user/windows
cargo test --locked --test linux_raw_mft --no-run
```

The smoke test is ignored by default. Use the exact executable filename printed
by Cargo:

```sh
sudo env NTFS_MFT_TEST_VOLUME=/media/user/windows ./target/debug/deps/linux_raw_mft-EXACT_HASH --ignored --nocapture
```

Repeat with `NTFS_MFT_TEST_VOLUME=/dev/sdXN` for direct-device access. Test support
recognizes paths under `/dev` as devices and other paths as mount roots; use
`Volume::from_device_path` directly for image files outside `/dev`.

The smoke test scans up to 128 entries, requires a nonempty result, and rereads
the first entry to compare its file reference. It does not validate parallel
scanning or Windows API parity. Selected tests fail on setup or scan errors;
see the [live-test requirements](../CONTRIBUTING.md#live-device-tests).
