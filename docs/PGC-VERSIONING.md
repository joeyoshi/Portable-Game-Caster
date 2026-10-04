# Portable Game Caster Versioning Policy

PGC uses three independent version concepts.

## 1. Client product version
Semantic version shared across all client platforms that claim feature parity.

Example:

```text
PGC Client 0.6.0
- macOS
- Windows
- Linux
```

A platform should only claim the shared version when it implements the feature/behavior baseline associated with that version.

During pre-1.0 development:
- `0.MINOR.PATCH`
- MINOR = meaningful feature baseline change
- PATCH = compatible fixes/small improvements

## 2. Host product version
Independent Semantic Version lineage for Host applications.

Example:

```text
PGC Host 0.3.0
- Windows
- Linux
- macOS
```

Host and Client versions do not need to match.

## 3. Protocol version
Independent compatibility contract between Host and Client.

Example:

```text
Client 0.8.0 supports protocol v1
Host 0.4.2 supports protocol v1
=> compatible
```

Future example:

```text
Client 1.3.0 supports protocol v1-v2
Host 0.9.0 supports protocol v2
=> compatible
```

Do not infer protocol compatibility from product SemVer.

## Build revisions
Each platform artifact may also have its own monotonically increasing build number.

Example:

```text
macOS Client 0.7.2 (build 38)
Windows Client 0.7.2 (build 22)
Linux Client 0.7.2 (build 14)
```

Product version communicates feature behavior. Build number identifies the concrete artifact.

Platform-only packaging/build fixes do not necessarily require a shared SemVer bump if the shared product behavior remains unchanged.

Meaningful user-facing fixes to the product contract should normally increment the shared patch version.

## Platform mappings
macOS:
- `CFBundleShortVersionString` = Client/Host SemVer
- `CFBundleVersion` = platform build number

Windows:
- Product Version = Client/Host SemVer
- File/build version = platform build number

Linux/package systems:
- map product SemVer and package release/build fields appropriately.

## Debug/About output
Recommended client output:

```text
Portable Game Caster Client
Version 0.6.0
Build 42
Platform: macOS arm64
Protocol: 1
```

Recommended Host output:

```text
Portable Game Caster Host
Version 0.3.0
Build 18
Platform: Windows x64
Protocol: 1
```
