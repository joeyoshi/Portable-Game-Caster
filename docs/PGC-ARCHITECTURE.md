# Portable Game Caster Architecture

## 1. Purpose
Portable Game Caster is a local-network streaming/capture system intended to make console gameplay easy to view/share from another device without requiring a conventional capture/streaming workflow each time.

The initial real-world target is:
- console/AVR/capture hardware near a Windows desktop,
- capture/encode/relay on Windows,
- low-latency LAN playback on a MacBook,
- Discord app/window sharing from the Mac.

Long-term targets include Windows, macOS, Linux, Steam Deck, Android, iOS, and other practical client/host platforms where the architecture makes sense.

## 2. Current data path

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
macOS PGC Client
        |
        v
ffplay
        |
        v
Discord share
```

## 3. Windows host

### 3.1 Current host supervisor
A Rust `pgc-host-windows` executable now:
- starts MediaMTX,
- advertises PGC over mDNS,
- monitors MediaMTX,
- restarts MediaMTX if it dies,
- shuts down cleanly.

This replaced the earlier need to remember to start discovery and MediaMTX separately.

### 3.2 FFmpeg ownership
FFmpeg is still launched through MediaMTX `runOnDemand` using PowerShell scripts and a PID file. This is temporary.

Desired end state:

```text
PGC Host
|- MediaMTX ownership
|- FFmpeg ownership
|- discovery advertisement
|- capture-device detection
|- encoder/backend selection
|- media health/status
|- automatic recovery
`- service/background lifecycle
```

### 3.3 Capture prototype
Current prototype capture hardware: Elgato HD60 S+.

Video input:
- `Game Capture HD60 S+`
- raw YUY2/yuyv422
- 1920x1080 @ 60 fps

Audio input:
- `Digital Audio Interface (Game Capture HD60 S+)`
- PCM s16le
- 44.1 kHz stereo

Audio workaround uses Denon AVR Zone 2 analog output so the AVR can keep speaker playback while capture receives stereo audio.

### 3.4 FFmpeg pipeline
Current encode approach:
- format conversion to NV12
- H.264 NVENC
- 1080p60
- ~20 Mbps CBR
- low-latency preset/tuning
- no B frames
- AAC 192 kbps / 48 kHz
- async audio resampling
- GOP 30
- `dump_extra=freq=keyframe`

Windows FFmpeg is intentionally pinned to Gyan 8.0.1 due GTX 1080 Ti / Pascal NVENC driver/API compatibility.

### 3.5 MediaMTX
Current defaults:
- SRT: 8890
- RTSP: 8554
- stream path: `/gameplay`
- `readTimeout: 2s`
- `runOnDemandCloseAfter: 5s`

FFmpeg publishes SRT locally to:

```text
srt://127.0.0.1:8890?streamid=publish:gameplay&pkt_size=1316&latency=20000&tlpktdrop=1
```

macOS reads:

```text
rtsp://<host>:8554/gameplay
```

## 4. Discovery

### 4.1 mDNS
Service type:

```text
_pgc._tcp.local.
```

Instance:

```text
Portable Game Caster
```

Current TXT metadata:

```text
protocol=rtsp
path=/gameplay
version=1
```

The `version` field should become an explicitly named protocol version.

### 4.2 Discovery behavior
The client:
- browses for PGC services,
- tolerates service resolution before IPv4 resolution,
- resolves host/IP/port/path/protocol,
- uses targeted same-host rediscovery during recovery,
- must never silently switch to another host while reconnecting.

## 5. macOS client

### 5.1 Current app
The macOS app is a native AppKit shell written in Rust using `objc2` bindings.

Responsibilities:
- render state,
- expose Retry/Search/Quit actions,
- own the ffplay child process handle for application lifecycle,
- poll state messages from worker threads,
- remain responsive while networking/player work happens off-main-thread.

### 5.2 State semantics
Core rule: UI state describes observable reality.

```text
Idle
 -> Discovering
 -> Resolving
 -> Connecting
 -> WaitingForStream
 -> Playing
```

Failure/recovery:

```text
Playing
 -> ReconnectingStream / ReconnectingHost
 -> WaitingForStream
 -> Playing
```

or:

```text
... -> Error / Idle
```

Key definitions:
- `Connecting`: host/service handshake is being established.
- `WaitingForStream`: host/service is available, but actual media is not yet confirmed.
- `Playing`: decoded media flow has been confirmed.
- `ReconnectingStream`: previously healthy playback was lost while host/service may still exist.
- `ReconnectingHost`: the previously connected host itself is not currently discoverable/reachable.

Fresh/cold connection attempts must not use reconnect states.

### 5.3 ffplay integration
Current known-good playback behavior uses RTSP/TCP and low-latency ffplay flags.

ffplay stderr is captured so PGC can derive interim health signals:
- RTSP transport failure,
- media-start confirmation,
- basic stream metadata.

Normal mode should inspect internally without noisy output.
Debug mode should emit concise PGC logs.
Verbose mode should additionally tee raw ffplay output.

Long-term, ffplay should be wrapped/replaced by a dedicated PGC player window with a title such as:

```text
Portable Game Caster Stream — <hostname>
```

## 6. Logging / observability

Runtime modes:
- packaged app, no flag: Off
- raw development executable, no flag: Debug
- `--debug`: Debug
- `--verbose`: Trace + raw ffplay diagnostics
- `--quiet`: Off

Logging goals:
- every state transition,
- host/IP/URL discovery data,
- ffplay path/PID,
- transport/media health events,
- reconnect attempts/timing,
- dependency resolution,
- future media-flow diagnostics.

Long-term idea: integrated in-app log panel using an internal ring buffer, hidden in normal mode and visible in debug mode.

## 7. Versioning

Three independent versions:

### Client SemVer
Shared across all client platforms when feature parity is claimed.

### Host SemVer
Shared across all host platforms when feature parity is claimed.

### Protocol version
Independent compatibility contract between Host and Client.

Platform-specific build numbers may differ without changing shared product SemVer when only the build/artifact changes.

## 8. Future architecture direction

Shared Rust core can eventually own:
- protocol,
- discovery,
- state machine,
- logging,
- health model,
- compatibility,
- config.

Platform shells can remain native:
- macOS/iOS: AppKit/SwiftUI as appropriate,
- Windows: native Windows APIs/UI,
- Linux/Steam Deck: native Linux stack,
- Android: Kotlin shell around shared Rust core where useful.
