# Release checklist

This project publishes the same source as a GitHub release and a crates.io
package. Complete the checks below from a clean checkout with hydrated Git LFS
fixtures. Preparing a release does not authorize publishing it.

## Before publication

1. Confirm the repository is intended to be public. Review the full Git history,
   issues, pull requests, Actions logs, and release artifacts for credentials,
   personal filesystem metadata, private paths, or material without redistribution
   rights. Checking only the current tree is insufficient.
2. Confirm fixture provenance, hashes, privacy review, and licenses in
   `tests/fixtures/omerbenamram-mft/README.md` and
   `tests/fixtures/sparse-volume-fixture/README.md`. Do not replace fixture bytes
   without updating their provenance and independent expectations.
3. Confirm GitHub private vulnerability reporting is enabled, or verify the
   email fallback in `SECURITY.md`.
4. Confirm the `ntfs-mft` crate name is controlled by the intended crates.io
   account and that the account has a verified email and two-factor authentication.
5. Update `CHANGELOG.md`, remove the `unreleased` marker, and use the same version
   in `Cargo.toml`, the Git tag, the GitHub release, and crates.io.

## Validation

Run:

```text
git lfs pull
python tests/fixtures/sparse-volume-fixture/verify.py
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo doc --package ntfs-mft --no-deps --locked
cargo package --locked
cargo publish --dry-run --locked
```

Review `cargo package --list` and inspect the generated `.crate` archive. Confirm
that the archive includes both fixture licenses and contains no build output,
credentials, personal logs, or unrelated binaries. Keep elevated device results
separate from portable tests and CI. Run live-device checks from `CONTRIBUTING.md`
when raw-volume behavior changed.

## Publish

Publication is an explicit maintainer action:

1. Merge the reviewed release commit and wait for required CI checks to pass.
2. Publish with `cargo publish --locked` and verify the crate page, README,
   license, repository link, and docs.rs build.
3. Create a signed `vX.Y.Z` tag from the published commit and push it.
4. Create the GitHub release from that tag using the matching changelog section.
5. Verify a new consumer can resolve the crates.io release on each supported
   platform. Record any platform checks that were not run.

Crates.io versions cannot be overwritten. If publication is wrong, yank the
affected version, document why, and publish a corrected version rather than
moving the Git tag or rewriting release history.
