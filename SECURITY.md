# Security policy

## Reporting a vulnerability

Please report suspected vulnerabilities privately to
[wangfu91@hotmail.com](mailto:wangfu91@hotmail.com), with `ntfs-mft security`
in the subject. If GitHub private vulnerability reporting is enabled, you can
also use [Report a vulnerability](https://github.com/wangfu91/ntfs-mft/security/advisories/new).
If that option is unavailable, use email; do not open a public issue with exploit
details or sensitive filesystem data.

Include the affected version or commit, platform, relevant options, expected
and actual behavior, and a minimal synthetic reproducer if possible. Describe
the impact and whether the input can be supplied by an untrusted party. Do not
send personal disk captures or credentials; arrange a suitable way to share
necessary evidence privately first.

Maintainers will coordinate investigation and disclosure with the reporter.
Response times depend on maintainer availability; no response-time guarantee is
made. Before the first release, report against the default branch. After
publication, try to reproduce on the latest release as well; older versions do
not have a separate maintenance commitment.

## Scope and trust boundaries

This library parses NTFS records from offline streams and live volumes and can
export metadata. Filesystem bytes, names, record references, attribute lengths,
runlists, and exported text may be attacker-controlled. A caller chooses the
input device or stream and the scan configuration. Raw-volume permissions are
provided by the operating system; this crate does not grant those permissions.

Security review should cover the parser, arithmetic and allocation decisions,
stream and volume I/O, unsafe code and FFI, path reconstruction, and exports.
Properties that must hold include:

- Volume access stays read-only.
- Untrusted offsets and lengths do not cause memory unsafety or unchecked
  arithmetic that bypasses validation.
- Malformed records do not cause unintended reads outside the supplied stream
  or captured ranges, or nonterminating parsing.
- Names and reconstructed paths remain data. They must not trigger execution
  or filesystem writes inside the library.
- Export preserves the documented distinction between known, unknown, and
  partial metadata.

Please report memory-safety issues, unintended writes or disclosure, and
input-triggered crashes or disproportionate resource exhaustion. Explain the
reachable call path and impact; ordinary processing costs proportional to a
large scan are useful context when assessing resource-exhaustion reports.

## Documented limitations

Live scans are not atomic snapshots. Historical and best-effort paths can be
partial; they are not authorization decisions or a sandbox for later file I/O.
Callers must validate paths before using them for other operations. Scan output
can contain private filesystem metadata and is not automatically anonymized.

These limitations provide context, not blanket exclusions for vulnerabilities.
See [the API guide](docs/api-guide.md) for specific completeness and error contracts.
