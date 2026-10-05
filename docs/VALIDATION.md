# Validation

Use [CONTRIBUTING.md](../CONTRIBUTING.md#validation) for commands and
[the release checklist](releasing.md) before publishing. The root `Cargo.lock`
is tracked and shared by both test harnesses.

## What each check establishes

| Check | Evidence | Limits |
| --- | --- | --- |
| Root tests | Parser, metadata, synthetic I/O, fixture and public API behavior | Does not open live devices by default |
| Differential harness | Comparison with the pinned independent parser | Uses extracted/synthetic bytes, not native device I/O |
| Windows parity harness | Portable decoder tests; opt-in comparison with Windows APIs | Live parity requires a configured NTFS drive and raw-volume permissions |
| Native CI matrix | Build, tests, Clippy, rustdoc and package verification on Windows, Linux and macOS | Device tests stay ignored; check the actual run status |
| Explicit live-device tests | Behavior on the selected device, OS and launch context | Does not establish other platform, filesystem or hardware coverage |
| Package and publish dry run | Archive construction and verification build | Does not publish, validate account ownership, or build docs.rs remotely |

Device tests are `#[ignore]` by default. Once selected with `--ignored`, missing
configuration, failed opens, unexpected filesystems, scan errors, and insufficient
comparison data are failures. An ignored test is not a successful device check.

## Recorded evidence

Each report is a snapshot. Keep its revision, platform, commands and limitations
with its results; do not infer current CI or release readiness from an older run.

- [September 2026 correctness and security audit](audits/2026-09-21-correctness-security.md)
- [October 2026 release review and outstanding checks](audits/2026-10-05-release-readiness.md)
- [September 2026 parser performance measurements](audits/2026-09-21-performance.md)
- [Earlier platform validation](audits/historical-platform-validation.md)
- [Logical-stream test coverage and earlier Windows smoke test](mft-stream-validation.md)

For platform setup and launch restrictions, see [Windows raw-volume access](windows-raw-volume-access.md)
and [macOS raw-volume access](macos-raw-volume-access.md).
