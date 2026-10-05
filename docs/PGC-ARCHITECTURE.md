# Portable Game Caster Architecture

## 1. Purpose

Portable Game Caster is a local-network streaming/capture system intended to make console gameplay easy to view and share from another device without rebuilding a conventional capture/streaming workflow each time.

Initial real-world target:

- console / AVR / capture hardware near a Windows desktop
- capture, encode, and relay on Windows
- low-latency LAN playback on a MacBook
- Discord app/window sharing from the Mac

Long-term platform targets may include Windows, macOS, Linux, Steam Deck, Android, and iOS/iPadOS.

## 2. Architecture principles

### Truthful state

UI state describes observable reality. `Playing` requires confirmed decoded media. A running process, reachable port, RTSP handshake, or ffplay process alone is insufficient.

Fresh connections never use reconnect states. Reconnect language is reserved for recovery after a previously healthy Playing session. Once connectivity is restored, the Client returns to `WaitingForStream` until decoded media is confirmed again.

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
NVENC: idle
publisher traffic: none
```

A few seconds of first-stream startup is acceptable. Low idle resource use is a first-class product requirement.

### Process ownership

The native Host owns FFmpeg lifecycle: launch, process lifetime, restart, stop, capture/encoder health, and shutdown cleanup. MediaMTX is relay and demand-detection infrastructure, not encoder owner.

## 3. Media path

```text
Console / HDMI chain
-> capture device
-> Windows FFmpeg (DirectShow, H.264 NVENC, AAC)
-> SRT localhost
-> MediaMTX
-> RTSP/TCP over LAN
-> Portable Game Caster Client
-> ffplay / future native playback layer
-> Discord share / local viewing
```

## 4. Windows Host

The Windows Host currently owns:

- MediaMTX supervision
- mDNS advertisement
- machine-wide singleton protection
- native FFmpeg process ownership
- demand-state interpretation
- capture/encoder lifecycle and recovery
- shutdown cleanup
- diagnostics
- Windows Job Object containment for child infrastructure

Future Host responsibilities include capture-device enumeration, video/audio source selection, persisted configuration, encoder/backend selection, lightweight console controls, and richer capture/media health reporting.

### Singleton

Only one Host may run per Windows machine.

```text
Global\PortableGameCasterHost.v1
```

`.v1` identifies the singleton contract, not Host SemVer. Multiple Hosts on one LAN are valid.

### Demand signaling and FFmpeg ownership

MediaMTX `runOnDemand` launches:

```text
pgc-host-windows.exe --demand-signal
```

The helper connects to the already-running Host over localhost and holds the connection open. Open helper connection means reader demand exists; close means that demand signal ended. The helper never owns FFmpeg.

This preserves demand from arbitrary compatible RTSP readers without requiring a PGC-specific Client control protocol.

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

Teardown:

```text
last viewer leaves
-> MediaMTX waits 10s runOnDemandCloseAfter
-> demand helper exits
-> Host waits 5s encoder grace
-> Host sends q to FFmpeg
-> escalates only if needed
-> capture/NVENC return idle
```

Normal last-reader-to-idle is therefore about 15 seconds plus graceful FFmpeg exit time.

Unexpected FFmpeg exit while demand remains uses bounded restart backoff: 1s, 2s, then 5s; reset after about 10 seconds healthy.

MediaMTX failure clears demand, stops/confirms FFmpeg, restarts MediaMTX, and waits for fresh demand before encoding again.

### Job Object

MediaMTX and FFmpeg are assigned to a Host-owned kill-on-close Windows Job Object on a best-effort basis. Assignment failure warns rather than failing startup. Hard-killing Host should not leave owned children behind.

### Console and observability — active transition

Windows logging/console work is implemented but still awaiting native Windows acceptance on this branch.

Current intended Host console behavior:

- Normal exposes meaningful DEMAND / ENCODER / STREAM lifecycle
- publisher wait/availability and configured media are visible
- publisher-wait warnings at 5 / 15 / 30 seconds
- unexpected FFmpeg exit includes a best recent `Cause:` line
- demand-loss wording reports only what Host can actually observe
- `L` opens the logs folder
- console widening is best-effort

Known immediate issue: classic Windows QuickEdit/selection can pause apparent console progress. The Host must disable selection-induced suspension without breaking Ctrl+C or the `L` hotkey. Native validation is required before this console/logging state is accepted.

## 5. Capture prototype

Prototype capture device: Elgato HD60 S+.

Video:

```text
Game Capture HD60 S+
YUY2 / yuyv422
1920x1080 @ 60 fps
```

Audio:

```text
Digital Audio Interface (Game Capture HD60 S+)
PCM s16le / 44.1 kHz / stereo
```

Current AVR workaround uses Denon Zone 2 analog output so AVR speakers and stereo capture can coexist.

## 6. Encode pipeline constraints

Current Windows prototype uses Gyan FFmpeg 8.0.1 full, intentionally pinned for GTX 1080 Ti / Pascal compatibility.

Current characteristics:

- YUY2 -> NV12 before Pascal NVENC
- H.264 NVENC 1080p60
- about 20 Mbps CBR
- low/ultra-low latency tuning, zerolatency
- no B frames
- GOP 30
- AAC 192 kbps / 48 kHz stereo
- async resampling
- `-bsf:v dump_extra=freq=keyframe`

Do not use `-use_wallclock_as_timestamps 1`; it caused non-monotonic DTS. Do not suggest `-repeat_headers 1`; unsupported by the pinned build.

## 7. MediaMTX

Current version: `1.21.1`.

```text
SRT publisher: localhost:8890, streamid=publish:gameplay
RTSP reader:    :8554/gameplay
```

Relevant settings:

```text
readTimeout: 2s
writeTimeout: 10s
runOnDemand: $PGC_HOST_EXE --demand-signal
runOnDemandRestart: false
runOnDemandStartTimeout: 15s
runOnDemandCloseAfter: 10s
```

No active `runOnUnDemand` script.

Source inspection established that a waiting DESCRIBE is not counted as an active reader and close-after can expire while a Client is still waiting. Therefore MediaMTX demand semantics are insufficient to own PGC encoder recovery. Do not return FFmpeg lifecycle ownership to MediaMTX scripts.

## 8. Discovery

Service:

```text
_pgc._tcp.local.
```

Current TXT:

```text
protocol=rtsp
path=/gameplay
version=1
```

`version` currently means protocol compatibility and should eventually become an explicit `protocol_version` field.

Multiple Hosts on one LAN are valid. Current Client auto-connects to the Host it discovers. The planned first-release Host Browser will discover without auto-connecting and let the user select explicitly.

## 9. macOS Client

Native AppKit application in Rust using `objc2`.

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
- Quit always available

Cancel invalidates stale worker generations and terminates race-started ffplay. Stop terminates ffplay and returns to Idle without recovery. No Client -> Host cancel protocol exists.

### Interaction model — UX Approved

- primary action slot contains Search / Cancel / Stop / Retry
- Quit remains separate
- utility shelf contains Open Logs Folder
- no default button
- initial focus none
- Space / Return / Enter activate deliberately focused button once
- held key repeats cannot chain through replacement controls
- focus follows the logical primary slot across state changes; clears when no replacement exists
- `L` opens logs folder
- hover/pressed feedback exists on utility and primary controls

Development-only `--ui-self-test` exercises layout/key/focus behavior without discovery. `PGC_UI_SNAPSHOT_DIR` optionally renders snapshots. It is compiled out of release builds and is engineering validation, not UX acceptance.

## 10. ffplay and media health

Playback transport is RTSP/TCP. Known-good ffplay options include `nobuffer`, `low_delay`, `noinfbuf`, `framedrop`, `sync ext`, probe/analyze settings, and `max_delay 0`.

`Playing` requires confirmed decoded media from ffplay telemetry. After media starts, Client ignores the advancing `-sync ext` master clock and compares the remaining status fields. About five seconds of unchanged non-master progress emits a stall and enters normal recovery.

Known follow-ups: no-telemetry watchdog, separate video/audio liveness, and a first-class health model beyond text parsing.

## 11. Logging and observability

Status:

- macOS Client: UX Approved
- Windows Host: implemented, awaiting native Windows validation
- unified logging feature: not Done yet

Modes:

- **Quiet**: no terminal output; session file still written
- **Normal**: important lifecycle/user-facing production diagnostics
- **Debug**: structured event-driven Portable Game Caster diagnostics; no raw subprocess firehose
- **Verbose**: Debug plus raw external provenance and deeper internal state/decision details

Presentation rules:

- UTC/Zulu millisecond timestamps
- fixed-width source/category fields
- readable product-facing STATE lines at Normal/Debug
- raw internal enum/state only at Verbose
- terminal styling centralized; files/redirects plain text; honor `NO_COLOR`
- Verbose explicitly identifies `[PGC]`, `[FFPLAY]`, `[MTX]`, `[FFMPEG]`
- FFmpeg carriage-return progress normalized/throttled rather than corrupting structured lines

Startup header is 50 characters wide and includes title, build channel + SemVer, platform, logging mode, protocol, session ID, and log folder.

Session files:

```text
macOS:   ~/Library/Logs/Portable Game Caster/
Windows: %LOCALAPPDATA%\Portable Game Caster\Logs\
```

`PGC_LOG_DIR` overrides.

macOS Client accepted naming:

```text
pgc-client-latest.log
pgc-client-YYYY-MM-DDTHH-MM-SS.sssZ.log
```

Previous latest is archived using the session ID embedded in its own header. Newest five exact-pattern archives retained; renamed/copied files are outside rotation.

Windows Host currently uses timestamped per-launch logs; alignment with the Client `latest` scheme remains open.

The logging core is currently duplicated between Mac and Windows and is an obvious shared-crate candidate for the pre-release architecture audit, not an excuse for an immediate abstraction.

## 12. Versioning and application structure

Client SemVer, Host SemVer, protocol version, and platform build number are independent concepts. Build channel (`Development`, `Nightly`, `Beta`, `Release`) is provenance/presentation, not compatibility.

Host and Client remain separate applications and release artifacts. Shared crates/core are encouraged where justified; a unified Host+Client shell is not.
