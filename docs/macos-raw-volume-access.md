# macOS raw NTFS access

`Volume::from_mount_point("/Volumes/ESD-ISO")` resolves an exact mount root
through `statfs`, requires filesystem type `ntfs`, and opens the backing raw
character device read-only. Symlinks and trailing separators are canonicalized;
child directories are rejected. Path bytes are preserved without shell commands.

`Volume::from_device_path("/dev/diskNsM")` accepts a partition device or an image
whose byte zero is the NTFS boot sector. Use the explicit device for third-party
drivers reporting another filesystem type. `RawMft` validates the boot sector.

Mounted block devices can reject opens with `EBUSY`. The library maps
`/dev/diskNsM` to `/dev/rdiskNsM`, accepts explicit raw paths, and leaves image
paths unchanged. `DKIOCGETBLOCKSIZE` supplies alignment for buffered reads,
including bootstrapping. `device_path()` retains the supplied path for reopening.
Parallel workers and owned MFT streams reopen independent read-only sources.

## Permissions and testing

Raw-device permissions are separate from mounted-file permissions. `sudo` may
be needed to read the device; it does not change its permissions or enable writes.
Sandbox/privacy restrictions or exclusive device ownership can still block root.
Errors retain the OS error in `MftError::Io`; the library never elevates itself.
Offline files and images need only ordinary read permission.

Build as the ordinary user, then run in a context allowed to read the device:

```sh
cargo build --example raw_mft_serial_read
sudo ./target/debug/examples/raw_mft_serial_read /Volumes/ESD-ISO
cargo test --test macos_raw_mft --no-run
```

For the test, use the exact executable filename printed by Cargo:

```sh
sudo env USN_TEST_VOLUME=/Volumes/ESD-ISO ./target/debug/deps/macos_raw_mft-EXACT_HASH --nocapture
```

Repeat with `USN_TEST_VOLUME=/dev/diskNsM` for direct-device access. The test checks
serial scans, both parallel schedulers, root-record lookup, independent cursors,
and stream ownership. It skips without a selected source and fails on errors
when one is supplied. Use a stable volume: live scans are not atomic snapshots.
No mount, unmount, write, repair, or permission change is performed.

See Apple's [statfs contract](https://developer.apple.com/library/archive/documentation/System/Conceptual/ManPages_iPhoneOS/man2/statfs.2.html)
and [mounted-device check in spec_open](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/miscfs/specfs/spec_vnops.c).
