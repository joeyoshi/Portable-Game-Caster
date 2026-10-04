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

Client-side Cancel or Stop Stream are local Client actions rather than Host session-control commands.

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

The native Windows Host owns FFmpeg lifecycle.

MediaMTX is relay and demand-detection infrastructure; it does not own the encoder process.

The Host owns:

- FFmpeg launch
- FFmpeg process lifetime
- FFmpeg restart
- FFmpeg stop
- capture/encoder health
- cleanup on Host shutdown

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
        | RTSP/TCP over LAN
        v
PGC Client
        |
        v
ffplay / future native player layer
        |
        v
Discord share / local viewing
```

## 4. Windows Host

### 4.1 Responsibilities

The Windows Host currently owns:

- MediaMTX process supervision
- mDNS advertisement
- machine-wide single-instance protection
- FFmpeg process ownership
- demand-state interpretation
- capture/encoder lifecycle
- recovery
- shutdown cleanup
- diagnostics
- Windows Job Object containment for child infrastructure

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

### 4.3 Native FFmpeg ownership

Current architecture:

```text
PGC Host
|- MediaMTX supervisor
|- native FFmpeg owner
|- demand listener
|- --demand-signal helper mode
|- Windows Job Object
|- mDNS
|- recovery
`- lifecycle
```

MediaMTX `runOnDemand` launches:

```text
pgc-host-windows.exe --demand-signal
```

The helper connects to the running Host over localhost and holds that TCP connection open.

```text
helper connection open
= reader demand exists

helper connection closes
= that demand signal ended
```

This preserves the useful property that any compatible RTSP reader can create demand without requiring PGC-specific Client session commands.

The helper is not the encoder owner. The already-running Host remains authoritative.

Native Host ownership rules:

- Host directly launches FFmpeg.
- Host retains the child/process handle.
- Host guarantees no more than one owned FFmpeg.
- Host distinguishes expected stop from unexpected exit.
- Host restarts FFmpeg when demand remains.
- Host stops FFmpeg when demand ends.
- Host shutdown confirms FFmpeg and MediaMTX are gone.
- PID files are not the primary ownership model.
- PowerShell start/stop scripts are legacy/deprecated and are not part of the active lifecycle.

### 4.4 Demand lifecycle

Current behavior:

```text
no viewers
-> no FFmpeg

first viewer demand
-> MediaMTX starts --demand-signal helper
-> Host receives demand
-> Host launches FFmpeg
-> FFmpeg opens capture
-> FFmpeg publishes SRT
-> MediaMTX exposes stream
-> Client confirms media
-> Playing
```

Current teardown:

```text
last viewer leaves
-> MediaMTX waits 10s runOnDemandCloseAfter
-> demand helper exits
-> Host sees demand inactive
-> Host waits 5s encoder grace
-> Host sends q to FFmpeg
-> FFmpeg exits gracefully when possible
-> capture/NVENC return idle
```

Normal last-reader-to-idle time is therefore approximately 15 seconds plus graceful FFmpeg shutdown time.

FFmpeg recovery:

```text
FFmpeg fails while demand exists
-> Host notices exit
-> Host backs off briefly
-> Host relaunches FFmpeg
-> MediaMTX receives new publisher
-> Client recovery continues
```

Current restart backoff is 1s, then 2s, then 5s, with reset after a healthy run of approximately 10 seconds.

MediaMTX failure:

```text
MediaMTX fails
-> Host detects exit
-> Host clears all demand signals
-> Host stops and confirms FFmpeg
-> Host restarts MediaMTX
-> no encoder runs until fresh reader demand exists
```

### 4.5 Job Object containment

The Host creates a Windows Job Object configured for kill-on-close and assigns MediaMTX and FFmpeg to it on a best-effort basis.

Purpose:

- hard-killing the Host should also terminate owned child infrastructure
- shutdown should not leave encoder/relay orphans

Failure to assign a child to the Job Object is currently treated as a warning rather than a Host startup failure.

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

Current relevant settings:

```text
readTimeout: 2s
writeTimeout: 10s
runOnDemand: $PGC_HOST_EXE --demand-signal
runOnDemandRestart: false
runOnDemandStartTimeout: 15s
runOnDemandCloseAfter: 10s
```

There is no active `runOnUnDemand` script.

### 7.1 MediaMTX demand semantics

MediaMTX 1.21.1 source was inspected during recovery debugging.

Important verified behavior:

- a DESCRIBE waiting for a source does not count as an active reader
- publisher loss does not itself reset all on-demand state
- close-after can expire while a Client is still waiting for recovery
- therefore MediaMTX's internal demand model is not sufficient to own encoder recovery semantics

The current architecture keeps MediaMTX responsible only for starting/stopping the lightweight demand helper. The Host owns FFmpeg recovery while the helper remains connected.

If real-world FFmpeg recovery ever regularly exceeds the current MediaMTX close-after window, the preferred follow-up is for the Host to consult MediaMTX reader state/control API before treating demand as truly gone. Returning FFmpeg lifecycle ownership to MediaMTX is not the preferred solution.

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

- `Connecting`: establishing host/service handshake
- `WaitingForStream`: host/service is reachable but media is not yet confirmed
- `Playing`: decoded media is confirmed
- `ReconnectingStream`: playback was previously healthy and stream connectivity has been lost
- `ReconnectingHost`: previously connected Host is no longer discoverable/reachable

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

The master clock continues advancing during a frozen stream and is therefore not itself a liveness signal.

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
- explicit user actions and final state transitions should be logged so diagnostic history matches observable UI state

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
