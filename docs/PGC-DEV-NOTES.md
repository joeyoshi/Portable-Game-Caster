# Portable Game Caster Development Notes

## Windows FFmpeg pin
Current prototype host uses Gyan FFmpeg 8.0.1 full build.

Reason: later builds tested required NVENC API 13.1 / driver >= 610, while the GTX 1080 Ti system currently exposes NVENC API 13.0.

Do not casually upgrade this dependency without retesting Pascal compatibility.

## Capture inputs
DirectShow video:

```text
Game Capture HD60 S+
```

DirectShow audio:

```text
Digital Audio Interface (Game Capture HD60 S+)
```

Input characteristics observed:
- video: YUY2/yuyv422, 1920x1080, 60 fps
- audio: PCM s16le, 44.1 kHz, stereo

Pascal NVENC cannot directly ingest the current YUY2/4:2:2 input, so conversion to NV12 is required.

## FFmpeg behavior to preserve
- Do not use `-use_wallclock_as_timestamps 1`; it produced non-monotonic DTS behavior.
- Use async resampling to 48 kHz.
- `-repeat_headers 1` is not supported by this h264_nvenc build.
- `-bsf:v dump_extra=freq=keyframe` plus `-g 30` substantially reduces cold-start PPS warnings but does not eliminate them.
- Known cold-start messages such as `non-existing PPS 0 referenced` remain a deferred cleanup item.

## Ingest experiments
### SRT localhost ingest
Current default and best media behavior.

### RTSP/TCP ingest
Rejected after testing:
- much better teardown behavior,
- eliminated PPS warnings,
- but caused severe DirectShow real-time-buffer backpressure and choppy/warped playback.

### RTSP/UDP ingest
Rejected:
- RTP packet loss,
- invalid FU-A packet errors,
- consistently choppy playback.

Do not casually revisit RTSP ingest unless the underlying issue is addressed with a fundamentally different approach.

## MediaMTX timing
Previously the default `readTimeout: 10s` caused stale SRT publisher state to persist after shutdown and poison rapid reconnects.

Current settings:

```yaml
readTimeout: 2s
writeTimeout: 10s
runOnDemandCloseAfter: 5s
```

Observed shutdown sequence:
- RTSP reader closes,
- 5s later runOnDemand stops / runOnUnDemand starts,
- roughly 2s later SRT closes EOF.

## Current PowerShell bridge
Temporary host flow:
- MediaMTX `runOnDemand` launches `start-gameplay.ps1`.
- PowerShell starts FFmpeg and records PID.
- `stop-gameplay.ps1` reads PID, verifies process name, force-stops FFmpeg, and removes PID file.

This should be replaced by native Host ownership.

## mDNS
Windows advertiser is implemented in Rust with `mdns-sd`.

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

macOS successfully resolves an IPv4 address through `enable_addr_auto()`.

## macOS ffplay
Current Homebrew requirement: `ffmpeg-full`.

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

`-sync ext` fixed playback pacing.

`-probesize 2M -analyzeduration 500000` was needed for reliable AAC/rate detection.

`-noinfbuf -max_delay 0` removed the previous manual seek/pause workaround.

Software decode is fine on Apple M4.

## Current UX rules
- Quit PGC must kill ffplay.
- Closing ffplay manually returns to Idle.
- Idle exposes Search for Host.
- Connected must eventually mean confirmed media flow.
- Reconnect must stay pinned to the same host.
- Fresh searches must not use reconnect states.

## Testing matrix worth preserving
Golden path:
- Host running,
- Mac discovers immediately,
- stream opens,
- media confirmed,
- ffplay manual close -> Idle,
- Search for Host works,
- Quit cleans player lifecycle.

Failure/recovery:
- discovery off,
- MediaMTX off,
- FFmpeg killed,
- MediaMTX killed,
- entire Host killed,
- reconnect within grace period,
- reconnect timeout,
- missing dependencies.
