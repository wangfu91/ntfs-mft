# Windows raw-volume access and MSIX

Live NTFS access requires both administrator privileges and an execution context
that permits raw-volume I/O. Opening a volume handle successfully does not
guarantee that Windows will allow reads through it.

## MSIX runtime model

An MSIX desktop app running as `packagedClassicApp` can retain a Windows
filesystem isolation Silo. Its child processes can inherit that context, including
children elevated through `sudo` or `RunAs`. Windows Container Isolation
(`wcifs.sys`) can reject user-mode raw-volume reads in that context with
`STATUS_NOT_SUPPORTED` (`0xC00000BB`), surfaced as Win32 error 50 or HRESULT
`0x80070032`. This is an execution-environment restriction, not an MFT parser or
extent-mapping defect.

For an MSIX application that needs live raw-volume access, use the appropriate
Win32 runtime declaration:

```xml
<Application Id="App" Executable="YourApp.exe"
             uap10:RuntimeBehavior="win32App"
             uap10:TrustLevel="mediumIL">
  <!-- Application elements -->
</Application>
```

Declare `xmlns:uap10="http://schemas.microsoft.com/appx/manifest/uap/windows10/10"`
on `Package` and include `uap10` in `IgnorableNamespaces`. This namespace requires
Windows 10 version 2004 (build 19041) or later. Do not retain a conflicting
`EntryPoint="Windows.FullTrustApplication"`: Microsoft defines that legacy
declaration as equivalent to `packagedClassicApp` with `mediumIL`.
`mediumIL` describes the trust model; it does not itself grant administrator
privileges. Validate the complete package and elevation flow on supported systems.

See Microsoft's [Application manifest reference](https://learn.microsoft.com/en-us/uwp/schemas/appxpackage/uapmanifestschema/element-f-application)
and [packaged desktop app runtime models](https://learn.microsoft.com/en-us/windows/msix/desktop/desktop-to-uwp-behind-the-scenes).
The `win32App` solution was also reported by the library user from their
SpaceRover application; we did not modify or repackage Codex to test that change.

## Diagnosing a library failure

The restriction can surface in `Volume::raw_mft()` / `RawMft::new()` while reading
the boot sector, before `MftByteReader` is constructed. Administrator checks,
package-identity queries and a query of only the innermost Job are not sufficient
preflight checks. Test an actual raw-volume read in the deployed launch context.

The same error code can originate from other drivers or unsupported operations;
it does not uniquely identify MSIX or `wcifs`. Likewise, being packaged as MSIX
or belonging to an ordinary Job alone does not establish that reads will fail.

An independently launched administrator terminal is a useful comparison.
`MftStream` over an already exported ordinary MFT file or memory does not perform
this raw-volume I/O; normal file permissions still apply.

## Verified Windows 11 case

On the investigated machine, Codex's installed MSIX manifest used
`EntryPoint="Windows.FullTrustApplication"` without `RuntimeBehavior="win32App"`.
The Windows Appinfo service held the package's outer Silo Job; both Codex and its
sudo-elevated PowerShell child were verified members. An inner ordinary Job had
initially hidden that relationship from a current-Job-only query.

Process Monitor showed matching read parameters between successful external and
failing Codex-launched probes. Minifilter tracing identified `wcifs!WcPreRead`,
and Wcifs telemetry recorded `BlockedOperation` with `OperationType=2` and
`OperationValue=3`. The installed driver's corresponding branch explicitly
returns `STATUS_NOT_SUPPORTED` for isolated user-mode raw-volume reads.

An independent administrator PowerShell, and an independent ordinary PowerShell
using `sudo --inline`, both read the NTFS boot sector successfully. Adding an
unrestricted Job to the independent administrator process also succeeded.
Restoring default UAC notifications did not change the failing inherited-Silo
case. No library workaround or system isolation change is needed for this finding.
