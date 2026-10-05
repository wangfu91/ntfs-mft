# Release checklist

Prepare and validate locally before changing GitHub visibility or publishing.
Publication and account settings are explicit maintainer actions.

## Review the release

- Review the current tree **and Git history**, issues, pull requests, Actions
  logs, and existing artifacts for credentials, private data, and redistribution
  concerns. Removing a file from the current tree does not remove its history.
- Verify fixture hashes and review the [upstream fixture](../tests/fixtures/omerbenamram-mft/README.md)
  and [sparse volume fixture](../tests/fixtures/sparse-volume-fixture/README.md)
  provenance. Preserve the library and upstream MIT notices. The sparse fixture
  contains Microsoft-origin bytes; this crate's MIT license grants no additional
  rights to those bytes. Do not assume an upstream sample is anonymized.
- Verify the private reporting email in [SECURITY.md](../SECURITY.md). After
  making the repository public, enable GitHub private vulnerability reporting if
  desired and check its reporting link.
- Check crate-name availability and the intended publishing account. A name is
  allocated on first publication, not reserved by a successful dry run. Verify
  the account email and use an appropriately scoped publishing token or trusted
  publishing configuration. Never put credentials in the repository or logs.
- Confirm the version in `Cargo.toml` and `Cargo.lock`. At release time, date the
  matching `CHANGELOG.md` section and remove its `unreleased` marker. Use that
  version for the tag and GitHub release too.

## Validate a clean checkout

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

Set `RUSTDOCFLAGS=-D warnings` for rustdoc. Run a current dependency advisory
check, such as `cargo audit`, and review findings in the affected dependency's
context. The workspace includes the pinned parser comparison and Windows parity
harnesses; they are not published.

Inspect `cargo package --list` and the generated `target/package/ntfs-mft-VERSION.crate`:

- Source, examples, fixture bytes, provenance and required notices are present.
- No Git LFS pointers, credentials, build output or unrelated files are included.
- The normalized package manifest has no unpublished runtime dependencies.

Test the extracted package as well as the checkout when changing package
contents. Cargo's packaging verification builds the library; it does not run
its tests or examples. Keep local, CI, and live-device results separate.
Use [CONTRIBUTING.md](../CONTRIBUTING.md#validation) for opt-in device checks.

## Publish

1. Merge the reviewed release commit and confirm all required native CI jobs
   actually passed. A job blocked by account billing or runner setup provides
   no platform validation.
2. Make the repository public when approved and check its README, links,
   description, license and security contact as an unauthenticated visitor.
3. From the validated clean commit, run `cargo publish --locked`. Verify the
   crate page and docs.rs build before announcing the release.
4. Tag that exact commit as `vX.Y.Z` (sign it when signing is configured), push
   the tag, and create a GitHub release using the matching changelog section.
5. Verify a fresh consumer resolves the published crate on each supported
   platform. Record any checks not run.

A dry run does not upload a crate or prove permission to publish it. Crates.io
versions cannot be overwritten; yanking a version does not erase its contents.
See Cargo's [publishing guide](https://doc.rust-lang.org/cargo/reference/publishing.html)
and [`cargo publish` reference](https://doc.rust-lang.org/cargo/commands/cargo-publish.html).
