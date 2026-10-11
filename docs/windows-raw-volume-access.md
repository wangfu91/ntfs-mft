# Windows raw-volume access

Live NTFS access requires administrator privileges and a launch context that
permits raw-volume I/O. A successful handle open does not guarantee readable data.

## MSIX runtime

A `packagedClassicApp` can retain a filesystem isolation Silo that its children
inherit, including processes elevated through `sudo` or `RunAs`. In that context,
`wcifs.sys` can reject raw reads with `STATUS_NOT_SUPPORTED` (`0xC00000BB`),
Win32 error 50, or HRESULT `0x80070032`.

For an MSIX app requiring raw-volume access, use the Win32 runtime declaration:

```xml
<Application Id="App" Executable="YourApp.exe"
             uap10:RuntimeBehavior="win32App"
             uap10:TrustLevel="mediumIL">
  <!-- Application elements -->
</Application>
```

Declare `xmlns:uap10="http://schemas.microsoft.com/appx/manifest/uap/windows10/10"`
on `Package` and add `uap10` to `IgnorableNamespaces`. This requires Windows 10
build 19041 or later. Remove a conflicting
`EntryPoint="Windows.FullTrustApplication"`, which implies `packagedClassicApp`
with `mediumIL`. `mediumIL` does not grant administrator privileges; validate the
packaged app's elevation and actual read behavior.

See Microsoft's [Application manifest reference](https://learn.microsoft.com/en-us/uwp/schemas/appxpackage/uapmanifestschema/element-f-application)
and [desktop runtime models](https://learn.microsoft.com/en-us/windows/msix/desktop/desktop-to-uwp-behind-the-scenes).

## Troubleshooting

Test an actual raw read in the deployed launch context, then compare it with an
independently opened administrator terminal. Elevation, package identity, or an
innermost-Job query alone cannot establish access. These error codes can also
come from other drivers; neither MSIX packaging nor Job membership alone proves
the cause. Exported MFT files use ordinary file I/O instead.

See [live-device testing](../CONTRIBUTING.md#live-device-tests) for test commands.
