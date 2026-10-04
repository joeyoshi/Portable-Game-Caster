# Portable Game Caster Architecture

## 1. Purpose

Portable Game Caster is a local-network streaming/capture system intended to make console gameplay easy to view and share from another device without requiring a conventional capture/streaming workflow every time.

Initial real-world target:

- console / AVR / capture hardware near a Windows desktop
- capture, encode, and relay on Windows
- low-latency LAN playback on a MacBook
- Discord app/window sharing from the Mac

Long-term targets may include:

- Windows
- macOS
- Linux
- Steam Deck
- Android
- iOS / iPadOS

## 2. Architecture principles

### Truthful state

UI state describes observable reality.

`Playing` means decoded media flow has been confirmed.

The following alone are insufficient:

- process exists
- TCP port is reachable
- RTSP handshake succeeds
- ffplay is running

### Broadcast/listener model

PGC is a broadcast/listener architecture.

Clients consume a stream. They do not tightly own Host sessions.

Client-side Cancel or Stop Stream should remain local Client actions rather than sending session-control commands to the Host.

### Demand-driven Host

Host capture is demand-driven by design.

When no viewer demand exists:

```text
PGC Host: running
MediaMTX: running
mDNS: advertising
FFmpeg: stopped
capture device: unopened
NVENC: idle
gameplay publisher traffic: none
```

A few seconds of first-stream startup is acceptable.

Low idle resource use is more important than instant first-frame startup.

### Process ownership

MediaMTX is relay/service infrastructure.

The native Host should own:

- FFmpeg launch
- FFmpeg process lifetime
- FFmpeg restart
- FFmpeg stop
- capture/encoder health
- cleanup on Host shutdown

MediaMTX should not be the long-term owner of FFmpeg lifecycle.

## 3. Media data path

```text
Console / HDMI chain
        |
        v
Capture device
        |
        v
Windows FFmpeg
  - DirectShow input
  - H.264 NVENC
  - AAC audio
        |
        | SRT localhost
        v
MediaMTX
        |
        | RTSP/TCP LAN
        v
PGC Client
        |
        v
ffplay / future native player layer
        |
        v
Discord share / local viewing
```

The media topology remains valid while FFmpeg process ownership moves from MediaMTX/PowerShell to the Rust Host.

## 4. Windows Host

### 4.1 Responsibilities

The Windows Host currently owns or is expected to own:

- MediaMTX process supervision
- mDNS advertisement
- machine-wide single-instance protection
- FFmpeg process ownership
- demand-state interpretation
- capture/encoder lifecycle
- recovery
- shutdown cleanup
- diagnostics

Future responsibilities include:

- capture-device enumeration
- audio/video source selection
- encoder/backend selection
- persisted settings
- Host UI/control surface
- capture/media health reporting

### 4.2 Single-instance behavior

Only one Host may run per Windows machine.

Current named mutex:

```text
Global\PortableGameCasterHost.v1
```

The `.v1` suffix is not tied to product SemVer.

It represents the singleton contract and should only change intentionally if PGC ever permits multiple incompatible Host generations on one machine.

Multiple PGC Hosts on the LAN are valid.

### 4.3 Native FFmpeg ownership transition

Current committed implementation may still contain the transitional:

```text
MediaMTX
-> runOnDemand
-> PowerShell
-> FFmpeg
```

bridge.

Approved target:

```text
PGC Host
|- MediaMTX
|- FFmpeg
|- demand interpretation
|- mDNS
|- recovery
`- lifecycle
```

Native Host ownership requirements:

- Host directly launches FFmpeg.
- Host retains the child/process handle.
- Host guarantees no more than one owned FFmpeg.
- Host distinguishes expected stop from unexpected exit.
- Host restarts FFmpeg when demand remains.
- Host stops FFmpeg when demand ends.
- Host shutdown confirms FFmpeg and MediaMTX are gone.
- PID files are not the primary ownership model.

### 4.4 Demand lifecycle

Desired behavior:

```text
no viewers
-> no FFmpeg

first viewer demand
-> Host launches FFmpeg
-> FFmpeg opens capture
-> FFmpeg publishes SRT
-> MediaMTX exposes stream
-> Client confirms media
-> Playing

last viewer leaves
-> short intentional grace
-> Host stops FFmpeg
-> capture/NVENC return idle
```

Recovery:

```text
FFmpeg fails while demand exists
-> Host notices exit
-> Host relaunches FFmpeg
-> MediaMTX receives new publisher
-> Client recovery continues
```

MediaMTX failure:

```text
MediaMTX fails
-> Host detects exit
-> Host restores clean relay state
-> Host restarts MediaMTX
-> demand/publisher lifecycle resumes without stale processes
```

## 5. Capture prototype

Current prototype hardware:

```text
Elgato HD60 S+
```

Video input:

```text
Game Capture HD60 S+
```

Observed video:

- YUY2 / yuyv422
- 1920x1080
- 60 fps

Audio input:

```text
Digital Audio Interface (Game Capture HD60 S+)
```

Observed audio:

- PCM s16le
- 44.1 kHz
- stereo

Current AVR audio workaround uses Denon Zone 2 analog output so speakers remain active while the capture device receives stereo audio.

## 6. FFmpeg encode pipeline

Current prototype target:

- FFmpeg Gyan 8.0.1 full build
- YUY2 -> NV12 conversion
- H.264 NVENC
- 1080p60
- approximately 20 Mbps CBR
- low-latency preset/tuning
- zero-latency behavior
- no B frames
- GOP 30
- AAC 192 kbps
- 48 kHz stereo
- async resampling
- `dump_extra=freq=keyframe`

Windows FFmpeg is intentionally pinned for GTX 1080 Ti / Pascal compatibility.

## 7. MediaMTX

Current version:

```text
1.21.1
```

Default endpoints:

- SRT: `8890`
- RTSP: `8554`
- path: `/gameplay`

FFmpeg publisher:

```text
srt://127.0.0.1:8890?streamid=publish:gameplay&pkt_size=1316&latency=20000&tlpktdrop=1
```

Client reader:

```text
rtsp://<host>:8554/gameplay
```

Current timing:

```text
readTimeout: 2s
writeTimeout: 10s
```

### Transitional runOnDemand behavior

The old bridge also uses:

```text
runOnDemandRestart: true
runOnDemandStartTimeout: 15s
runOnDemandCloseAfter: 5s
```

MediaMTX 1.21.1 source was inspected during recovery debugging.

Verified behavior:

- Windows external commands use a kill-on-close Job Object.
- Stopping runOnDemand terminates its PowerShell/FFmpeg command tree.
- Killing MediaMTX also closes that Job Object.
- runOnDemand restart uses a fixed approximately five-second pause.
- publisher loss does not reset the current on-demand state
- a DESCRIBE waiting for a source does not count as an active reader
- close-after can expire while the Client is still waiting for recovery
- start-timeout applies to a new command-start cycle

This mismatch between MediaMTX demand semantics and PGC Client recovery semantics is why MediaMTX-owned FFmpeg is being retired.

## 8. Discovery

Service:

```text
_pgc._tcp.local.
```

Instance:

```text
Portable Game Caster
```

Current TXT:

```text
protocol=rtsp
path=/gameplay
version=1
```

The generic `version` field should eventually become an explicitly named protocol-version field.

Client discovery rules:

- browse for PGC services
- tolerate service resolution before IPv4 resolution
- resolve host/IP/port/path/protocol
- recover against the same Host
- never silently switch to a different Host during reconnect

Multiple Hosts on one LAN are valid.

Host selection/preference is future Client UX.

## 9. macOS Client

### 9.1 Shell

The current macOS Client is a native AppKit application written in Rust using `objc2`.

Responsibilities:

- render truthful state
- provide contextual controls
- own ffplay
- keep UI work on the main thread
- move networking/player/reconnect work to workers
- receive state updates through channels

### 9.2 State model

Cold path:

```text
Idle
-> Discovering
-> Resolving
-> Connecting
-> WaitingForStream
-> Playing
```

Recovery:

```text
Playing
-> ReconnectingStream / ReconnectingHost
-> WaitingForStream
-> Playing
```

or:

```text
... -> Error
```

Definitions:

- `Connecting`
  - establishing host/service handshake
- `WaitingForStream`
  - host/service is reachable but media is not yet confirmed
- `Playing`
  - decoded media is confirmed
- `ReconnectingStream`
  - playback was previously healthy and stream connectivity has been lost
- `ReconnectingHost`
  - previously connected Host is no longer discoverable/reachable

Fresh connections never use reconnect states.

### 9.3 Session controls

- Idle: `Search for Host`
- Busy/recovery/warm-up: `Cancel`
- Playing: `Stop Stream`
- Error: `Retry`
- Quit remains available

Cancel:

- immediately returns to Idle
- invalidates stale worker generations
- terminates race-started ffplay

Stop Stream:

- terminates ffplay
- returns to Idle
- does not trigger recovery

There is intentionally no Client -> Host session-cancel protocol.

## 10. ffplay integration

Current playback transport:

```text
RTSP/TCP
```

Known-good flags:

```text
-rtsp_transport tcp
-fflags nobuffer
-flags low_delay
-noinfbuf
-framedrop
-sync ext
-probesize 2M
-analyzeduration 500000
-max_delay 0
-stats
```

ffplay stderr is used to derive:

- transport loss
- media-start confirmation
- stream metadata
- ongoing progress

## 11. Media health

Current rule:

```text
Playing = confirmed decoded media flow
```

After MediaStarted, the Client compares ffplay status information excluding the `-sync ext` master-clock column.

Reason:

The master clock continues advancing during a frozen stream.

Current stall behavior:

```text
non-master status unchanged for ~5s
-> TransportEvent::Stalled
-> terminate ffplay
-> existing recovery flow
```

Known follow-ups:

- no-telemetry watchdog if ffplay stops printing status entirely
- separate video and audio liveness tracking
- first-class media health rather than increasingly complex text parsing

## 12. Countdown timing

Visible timers are based on monotonic deadlines rather than worker-loop cadence.

This applies to:

- reconnect countdown
- warm-up fallback countdown
- discovery countdown behavior

The UI may add a small polling delay, but countdown values should advance on real second boundaries.

## 13. Logging and observability

### Target modes

#### Normal

- important lifecycle events
- warnings/errors
- minimal overhead
- no external raw output

#### Debug

- structured PGC diagnostics
- state transitions
- connection/discovery reasoning
- process lifecycle
- recovery/health information
- no raw external firehose

#### Verbose

- Debug plus raw external output
- explicit source identity on every line
- higher I/O cost is acceptable

### Presentation

```text
Debug:
[07:50:41.098Z] [STATE]      Playing("carock-pgc.local") | Connected.
[07:50:52.209Z] [HEALTH]     Active stream transport lost

Verbose:
[07:50:41.098Z] [PGC]    [STATE]      Playing("carock-pgc.local") | Connected.
[07:50:54.335Z] [FFPLAY]             ...
[07:50:54.336Z] [MTX]                ...
[07:50:54.337Z] [FFMPEG]             ...
```

Rules:

- UTC/Zulu timestamp
- millisecond precision
- dim timestamp
- stable category/source colours
- fixed-width fields
- consistent message start column
- Debug may omit `[PGC]`
- Verbose always identifies source
- redirected output is plain text
- terminal styling is centralized

Long-term possibility:

- integrated in-app log panel backed by a ring buffer

## 14. Versioning

Three separate compatibility/version concepts:

### Client SemVer

Shared by Client platforms that claim the same feature baseline.

### Host SemVer

Independent Host product lineage.

### Protocol version

Independent wire/discovery compatibility contract.

Product SemVer must not be used as a substitute for protocol compatibility.

Platform build numbers may differ while sharing the same product SemVer.

## 15. Future shared core

A future shared Rust core may own:

- protocol
- discovery
- state machine
- logging
- media health
- configuration
- compatibility rules

Platform shells can remain native where that provides better UX.