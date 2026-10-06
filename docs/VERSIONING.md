# Portable Game Caster Versioning

Portable Game Caster keeps four version/provenance concepts separate:

1. shared product version
2. platform/application build number
3. protocol version
4. build channel

These answer different questions and should not be conflated.

## Shared product version

Host and Client share one Portable Game Caster product version representing the same integrated product baseline.

Example:

```text
PGC Host   0.1.1 Build 8
PGC Client 0.1.1 Build 12
Protocol   1
```

The product version describes the PGC baseline. The independent build numbers identify the concrete Host and Client artifacts.

### Pre-1.0 policy

PGC uses the SemVer-shaped form:

```text
0.MINOR.PATCH
```

with a deliberate pre-1.0 project policy:

- MAJOR remains `0` until PGC declares a stable `1.0.0` product contract.
- MINOR identifies a meaningful development/release generation or milestone.
- PATCH identifies an accepted integrated product revision.

This is intentionally not strict textbook SemVer patch semantics. Before 1.0, PATCH is used as an integrated-development revision number so MINOR remains useful for larger milestones.

## Platform/application build number

Each executable lane carries its own monotonically increasing build number.

Current lanes include:

- Windows Host
- macOS Client

Build numbers identify accepted executable checkpoints and are independent between lanes.

Example:

```text
Windows Host 0.1.1 Build 8
macOS Client 0.1.1 Build 12
```

Different build numbers are normal and do not imply a compatibility mismatch.

A documentation-only or planning change does not inherently create a new build.

## Protocol version

Protocol version is the Host/Client compatibility contract.

Do not infer compatibility from product version or build number.

Current mDNS TXT contains:

```text
version=1
```

This currently represents protocol compatibility, not the product version.

If this field is renamed in the future, prefer an explicit compatibility-oriented name such as:

```text
protocol_version=1
```

## Build channel

Build channel describes provenance/presentation rather than compatibility.

Conceptual channels are:

```text
Development
Nightly
Beta
Release
```

Current development builds report the `Development` channel explicitly.

Build channel does not replace product version, build number, or protocol version.

## Runtime identity

Runtime diagnostics keep these concepts visibly separate.

Current development output follows this shape:

```text
PORTABLE GAME CASTER HOST
Version:     Development (0.1.2)
Build:       1
Platform:    Windows x86_64
Logging:     Normal
Protocol:    1
```

and:

```text
PORTABLE GAME CASTER CLIENT
Version:     Development (0.1.2)
Build:       1
Platform:    macOS arm64
Logging:     Normal
Protocol:    1
```

The logging row reflects the active logging mode and may differ between terminal and session-file output.

## Platform mappings

### macOS

```text
CFBundleShortVersionString = shared product version
CFBundleVersion = macOS Client build number
```

The current macOS application bundle uses the same product/build identity shown at runtime.

### Windows

The Windows Host exposes product/build identity at runtime.

Native Windows file/version-resource metadata is not yet part of the current implementation and may be added as packaging/release work matures.

### Linux/package systems

Future Linux/package targets should map the same concepts into the appropriate package-manager fields without changing their meaning.

## Source of truth

Product/build identity is explicit and repository-controlled.

It is not derived from Git commit count, branch name, or wall-clock timestamp.

CI may later append artifact provenance, but should not replace the durable product/build concepts.

## Release artifacts

Host and Client remain separate applications and ship as separate role/platform artifacts, for example:

```text
PGC-Host-Windows-x64
PGC-Client-Windows-x64
PGC-Client-macOS-arm64
```

Exact artifact and tag conventions may evolve.

## Summary

```text
Shared product version
    shared across Host and Client
    identifies the integrated product baseline

Platform/application build number
    independent per executable lane
    identifies the concrete accepted artifact checkpoint

Protocol version
    shared compatibility contract
    changes only when protocol semantics require it

Build channel
    provenance/presentation metadata
    no compatibility meaning
```
