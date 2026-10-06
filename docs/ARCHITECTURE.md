# Portable Game Caster Architecture

## Purpose

Portable Game Caster is a local-network streaming/capture system for making captured gameplay easy to view on another device without requiring cloud routing or a remote-control stack.

The current implementation uses a Windows Host and a macOS Client.

## Architecture principles

### Truthful state

UI state describes observable reality. `Playing` requires confirmed decoded media. A running process, reachable port, RTSP handshake, or player process alone is insufficient.

Fresh connections never use reconnect states. Reconnect language is reserved for recovery after a previously healthy Playing session. Once connectivity returns, the Client goes back to `WaitingForStream` until decoded media is confirmed again.

### Broadcast/listener model

Clients consume a local broadcast. They do not tightly own Host sessions. Client Cancel and Stop Stream are local actions rather than Host session-control commands.

### Demand-driven Host

When no viewer demand exists:

```text
PGC Host: running
MediaMTX: running
mDNS: advertising
FFmpeg: stopped
capture device: unopened
encoder: idle
publisher traffic: none
```

A few seconds of first-stream startup is acceptable. Low idle resource use is a first-class product requirement.

### Process ownership

The native Host owns FFmpeg lifecycle: launch, process lifetime, restart, stop, capture/encoder health, and shutdown cleanup. MediaMTX is relay and demand-detection infrastructure, not encoder owner.

### LAN-local core

Core discovery, control, and media transport are local-network functions. Basic operation must not depend on an Internet connection.

## Media path

```text
captured source
-> capture device
-> Windows FFmpeg
-> SRT localhost
-> MediaMTX
-> RTSP/TCP over LAN
-> Portable Game Caster Client
-> playback layer
```

## Windows Host

The Windows Host currently owns:

- MediaMTX supervision
- mDNS advertisement
- machine-wide singleton protection
- native FFmpeg process ownership
- demand-state interpretation
- capture/encoder lifecycle and recovery
- shutdown cleanup
- diagnostics
- Windows Job Object containment for owned child infrastructure

### Singleton

Only one Host may run per Windows machine.

```text
Global\PortableGameCasterHost.v1
```

`.v1` identifies the singleton contract, not Host SemVer. Multiple Hosts on one LAN are valid.

The singleton must be acquired before Host-owned session resources are created. Rejected second instances exit without creating or rotating Host session logs.

### Demand signaling and FFmpeg ownership

MediaMTX `runOnDemand` launches:

```text
pgc-host-windows.exe --demand-signal
```

The helper connects to the already-running Host over localhost and holds the connection open. An open helper connection represents reader demand. The helper never owns FFmpeg.

This allows compatible RTSP readers to trigger demand without a PGC-specific Client control protocol.

### Demand lifecycle

```text
no viewers
-> no FFmpeg

first viewer
-> MediaMTX starts demand helper
-> Host receives demand
-> Host launches FFmpeg
-> capture opens
-> FFmpeg publishes SRT
-> MediaMTX serves RTSP
-> Client confirms decoded media
-> Playing
```

Normal teardown:

```text
last viewer leaves
-> MediaMTX close-after grace
-> demand helper exits
-> Host encoder grace
-> Host requests graceful FFmpeg exit
-> escalates only if needed
-> capture/encoder return idle
```

Unexpected FFmpeg exit while demand remains uses bounded restart backoff. MediaMTX failure clears demand, stops/confirms FFmpeg, restarts MediaMTX, and waits for fresh demand before encoding again.

### Child containment

MediaMTX and FFmpeg are assigned to a Host-owned kill-on-close Windows Job Object on a best-effort basis so abnormal Host termination does not leave owned infrastructure behind.

## Media relay

MediaMTX is the local relay and demand bridge.

Current topology:

```text
FFmpeg -> SRT localhost -> MediaMTX -> RTSP/TCP LAN -> Client
```

MediaMTX demand semantics are not authoritative for FFmpeg recovery/lifecycle, so encoder ownership remains in the native Host.

## Discovery

Service:

```text
_pgc._tcp.local.
```

Current TXT fields:

```text
protocol=rtsp
path=/gameplay
version=1
```

`version` currently represents protocol compatibility. Product SemVer must not be inferred from this field.

Multiple Hosts on one LAN are valid.

## macOS Client

The current Client is a native AppKit application in Rust.

Cold path:

```text
Idle -> Discovering -> Resolving -> Connecting -> WaitingForStream -> Playing
```

Recovery:

```text
Playing -> ReconnectingStream/ReconnectingHost -> WaitingForStream -> Playing|Error
```

Controls:

- Idle: Search for Host
- Busy/recovery: Cancel
- Playing: Stop Stream
- Error: Retry
- Quit remains available

Cancel invalidates stale work and terminates race-started playback. Stop terminates playback and returns to Idle without triggering recovery. There is no Client -> Host cancel protocol.

### Interaction contract

- primary action slot contains Search / Cancel / Stop / Retry
- Quit remains separate
- utility control opens the logs folder
- no default button
- initial focus is none
- Space / Return / Enter activate a deliberately focused button once
- held-key repeats must not chain through replacement controls
- focus follows the logical primary-action slot across state changes and clears when no replacement exists
- `L` opens the logs folder

## Media health

Playback uses RTSP/TCP. `Playing` requires confirmed decoded-media progress rather than transport connectivity alone.

If media progress stops after a previously healthy session, the Client exits `Playing` and enters the normal recovery path.

## Logging and observability

Host and Client use the same conceptual logging levels:

- **Quiet**: terminal silent; support/session logging remains available
- **Normal**: important lifecycle and user-facing production diagnostics
- **Debug**: structured internal diagnostics without raw subprocess firehose
- **Verbose**: Debug plus raw external provenance and deeper internal detail

Diagnostic timestamps use UTC/Zulu with millisecond precision so logs from multiple machines can be correlated directly.

Interactive terminal styling is presentation only. Redirected output and files remain plain text.

## Versioning and application structure

Host and Client share one product version for the integrated PGC baseline. Each executable lane maintains its own independent build number. Protocol version remains the compatibility contract, while build channel (`Development`, `Nightly`, `Beta`, `Release`) is provenance/presentation rather than compatibility.

Host and Client remain separate applications and release artifacts. Shared crates/core are encouraged where justified; a unified Host+Client shell is not.
