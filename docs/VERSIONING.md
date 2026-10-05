# Portable Game Caster Versioning

Portable Game Caster uses independent version concepts for products, protocol compatibility, and platform artifacts. These communicate different things and must not be conflated.

## Client product version

Client applications use Semantic Versioning across platforms that claim the same feature and behavior baseline.

Example:

```text
PGC Client 0.7.0
- macOS
- Windows
- Linux
```

A platform should only claim a shared Client version when it implements that baseline.

### Pre-1.0 guidance

Use:

```text
0.MINOR.PATCH
```

- MINOR: meaningful feature or behavior baseline change
- PATCH: compatible bug fixes, polish, or small improvements

## Host product version

Host applications use an independent Semantic Version lineage.

Example:

```text
PGC Host 0.4.2
PGC Client 0.8.0
```

Host and Client versions do not need to match.

## Protocol version

Protocol version is the compatibility contract between Host and Client.

Example:

```text
Client 0.8.0 supports protocol v1
Host 0.4.2 supports protocol v1
=> compatible
```

Do not infer protocol compatibility from Host or Client SemVer.

## Discovery metadata

Current mDNS TXT contains:

```text
version=1
```

This represents protocol compatibility, not Host or Client SemVer.

If this field is renamed in the future, prefer an explicit compatibility-oriented name such as:

```text
protocol_version=1
```

## Platform build revisions

Each platform artifact may use its own monotonically increasing build number.

Example:

```text
macOS Client 0.7.2 (build 38)
Windows Client 0.7.2 (build 22)
```

Product version communicates the shared feature/behavior baseline. Build number identifies the concrete platform artifact.

Platform-only packaging/build changes do not necessarily require a shared product SemVer bump if product behavior is unchanged. Meaningful user-facing fixes to the product contract should normally increment PATCH.

## Build channel

Build channel is shown alongside product SemVer and describes provenance, not compatibility.

Supported conceptual channels:

```text
Development
Nightly
Beta
Release
```

Build channel does not replace product SemVer, protocol version, or platform build number.

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

## Runtime/About output

Version/provenance output should distinguish at least:

```text
Product version
Platform build number, when available
Build channel
Platform/architecture
Protocol version
```

## Release artifacts

Host and Client are separate applications and ship as separate role/platform artifacts, for example:

```text
PGC-Host-Windows-x64
PGC-Client-Windows-x64
PGC-Client-macOS-arm64
```

Artifact naming and tag conventions may evolve, but role, platform, product version, and protocol compatibility remain distinct concepts.

## Summary

These values may change independently:

```text
Client SemVer
Host SemVer
Protocol version
Platform build number
Build channel
```
