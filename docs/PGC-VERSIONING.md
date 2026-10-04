# Portable Game Caster Versioning Policy

PGC uses three independent version concepts:

1. Client product version
2. Host product version
3. Protocol version

These communicate different things and must not be conflated.

## 1. Client product version

Semantic Version shared across Client platforms that claim the same feature/behavior baseline.

Example:

```text
PGC Client 0.7.0
- macOS
- Windows
- Linux
```

A platform should only claim that shared version when it implements the baseline associated with that version.

### Pre-1.0

Use:

```text
0.MINOR.PATCH
```

Guideline:

- MINOR
  - meaningful feature or behavior baseline change
- PATCH
  - compatible bug fixes, polish, or small improvements

Because PGC is pre-1.0, compatibility may still evolve rapidly, but version changes should remain deliberate and meaningful.

## 2. Host product version

Host applications use an independent Semantic Version lineage.

Example:

```text
PGC Host 0.3.0
- Windows
- Linux
- macOS
```

Host and Client product versions do not need to match.

Example:

```text
Client 0.8.0
Host 0.4.2
```

This is normal.

## 3. Protocol version

Protocol version is the independent compatibility contract between Host and Client.

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

Do not infer protocol compatibility from Client or Host SemVer.

## Discovery metadata

Current mDNS TXT contains:

```text
version=1
```

This currently represents protocol compatibility.

When protocol-version implementation work is scheduled, migrate toward an explicit name such as:

```text
protocol_version=1
```

Do not repurpose this field for Host or Client SemVer.

## Build revisions

Each platform artifact may have its own monotonically increasing build number.

Example:

```text
macOS Client 0.7.2 (build 38)
Windows Client 0.7.2 (build 22)
Linux Client 0.7.2 (build 14)
```

Product version communicates the feature/behavior baseline.

Build number identifies the concrete platform artifact.

Platform-only build or packaging changes do not necessarily require a shared product SemVer bump if the shared product behavior remains unchanged.

Meaningful user-facing fixes to the product contract should normally increment the shared PATCH version.

## Platform parity

Shared product SemVer implies a shared feature/behavior baseline.

A platform that has not yet implemented that baseline should not claim the newer shared version merely to keep numbers visually synchronized.

This applies independently to Client platforms and Host platforms.

## Platform mappings

### macOS

```text
CFBundleShortVersionString = Client or Host SemVer
CFBundleVersion = platform build number
```

### Windows

```text
Product Version = Client or Host SemVer
File/build version = platform build number
```

### Linux/package systems

Map product SemVer and package release/build revision into the appropriate package-manager fields.

## Debug / About output

Recommended Client output:

```text
Portable Game Caster Client
Version 0.7.0
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

## Version independence summary

These may all change independently:

```text
Client SemVer
Host SemVer
Protocol version
platform build number
```

For example:

```text
Client 0.9.1 build 54
Host 0.5.0 build 31
Protocol 1
```

is a perfectly valid PGC release configuration.