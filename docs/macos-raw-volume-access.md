# macOS raw NTFS access

`Volume::from_mount_point("/Volumes/ESD-ISO")` resolves an exact mount root
through macOS `statfs`, verifies the reported filesystem type is `ntfs`, and
opens its backing raw character device read-only. Symlinks and trailing
separators are canonicalized; a child directory is rejected. The implementation
preserves Unix path bytes and does not invoke `diskutil` or a shell.

Alternatively, use `Volume::from_device_path("/dev/diskNsM")`. Supply the NTFS
partition, not the whole disk containing a partition table. A partition image
with an NTFS boot sector at byte zero also works through this API. `RawMft`
validates the boot sector when it is constructed. For third-party NTFS drivers
that report another filesystem type, use the partition device explicitly;
mount lookup currently accepts the native `ntfs` type only. Mount lookup
does not guarantee access: permissions, privacy restrictions, or an exclusive
device owner can still prevent a raw open.

macOS rejects opening a mounted `/dev/diskNsM` block device with `EBUSY`,
even for reads. The library maps BSD block-device names to `/dev/rdiskNsM`
raw character devices; explicit raw paths are also accepted. It queries the
logical device block size with `DKIOCGETBLOCKSIZE` and aligns buffered reads
to at least that size, including the initial boot-sector read. The supplied
device path is retained by `device_path()` and reused when reopening. Ordinary
partition-image paths are left unchanged. The shared Unix reader uses positional
reads, and parallel workers and owned MFT byte streams reopen read-only sources.
All device opens use read-only access. No mount, unmount, repair, write, or
permission-changing operation is performed.

Reading files in Finder does not imply permission to read the backing device.
Raw access bypasses the mounted filesystem’s per-file access checks, so macOS
restricts the device node separately. For example, a device owned by `root:operator`
with mode `0640` is readable by root and members of `operator`, but not by other
users. `sudo` runs the test as root; it does not change the device permissions
or make the library open the device for writing. Offline MFT files and partition
images only need ordinary read permission and do not inherently require `sudo`.

Access failures retain their OS error
as `MftError::Io`; the library never elevates itself. Build as the ordinary user
and run the resulting executable in a context permitted to read the device.
Sandbox and macOS privacy restrictions can still apply to an elevated process.

```sh
cargo build --example raw_mft_serial_read
sudo ./target/debug/examples/raw_mft_serial_read /Volumes/ESD-ISO
```

For hardware validation, compile the integration test as the ordinary user:

```sh
cargo test --test macos_raw_mft --no-run
```

Cargo prints an `Executable ... (target/debug/deps/macos_raw_mft-...)` path.
Run that exact executable, substituting its full filename below:

```sh
sudo env USN_TEST_VOLUME=/Volumes/ESD-ISO ./target/debug/deps/macos_raw_mft-EXACT_HASH --nocapture
```

Repeat with `USN_TEST_VOLUME=/dev/diskNsM` to validate direct-device reopening.
The test checks serial scanning, both parallel schedulers with four workers,
root-record rereading, independent byte-stream cursors, and stream ownership
after dropping the original volume. It skips when the variable is absent and
fails on access or parse errors when a source is explicitly selected. Use an
idle or read-only volume for reproducible comparisons: a live scan is not an
atomic snapshot.

The ordinary macOS test suite also covers offline parsing, Unix export bytes,
native mount lookup rejection, malformed mount metadata, and read-only file
descriptors without requiring an NTFS device. CI includes a native macOS runner.
Apple Silicon and Intel macOS are included in the documentation target list.

Mount metadata fields follow Apple's
[statfs contract](https://developer.apple.com/library/archive/documentation/System/Conceptual/ManPages_iPhoneOS/man2/statfs.2.html).

The raw-device choice follows the mounted-block-device check in Apple’s
[`spec_open` implementation](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/miscfs/specfs/spec_vnops.c).
