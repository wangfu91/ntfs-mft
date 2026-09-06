# Extraction provenance

Extracted from `wangfu91/usn-journal-rs`, branch `feat/raw-mft-read`, commit
`0e57c8eed844b22e3b261a6823910932b3ad15ef`.

The MIT license and original attribution are retained. The raw reader, snapshot
and historical paths, examples, tests, and benchmark support move here. Volume,
file-reference, FILETIME, attribute, error, privilege, and optional live-path
helpers are independently maintained copies. Journal APIs and FSCTL enumeration
remain in the source repository with their existing refinements.

Imports now use `ntfs_mft`; errors/results are `MftError`/`MftResult`. Raw reader
names remain available under `raw_mft` and are also exported at the crate root.
Benchmark environment variables retain their `USN_` names for script compatibility.
The cross-repository comparison harness is documented in `integration-tests`.
