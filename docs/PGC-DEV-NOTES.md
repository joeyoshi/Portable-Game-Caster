# Portable Game Caster Development Notes

This file contains implementation knowledge, tested constraints, failed experiments, and technical landmines that should not need to be rediscovered.

## Windows FFmpeg pin

Current prototype Host uses:

```text
Gyan FFmpeg 8.0.1 full build
```

Reason:

Newer tested builds required NVENC API 13.1 / driver >= 610.

The current GTX 1080 Ti / Pascal system exposes NVENC API 13.0.

Do not casually upgrade FFmpeg without retesting Pascal compatibility.

## Capture inputs

DirectShow video:

```text
Game Capture HD60 S+
```

DirectShow audio:

```text
Digital Audio Interface (Game Capture HD60 S+)
```

Observed input:

- video:
  - YUY2 / yuyv422
  - 1920x1080
  - 60 fps
- audio:
  - PCM s16le
  - 44.1 kHz
  - stereo

Pascal NVENC cannot directly ingest the current YUY2/4:2:2 input.

Convert to NV12 first.

## Current FFmpeg behavior to preserve

Do not use:

```text
-use_wallclock_as_timestamps 1
```

It produced non-monotonic DTS behavior.

Current audio approach:

```text
aresample=48000:async=1000:first_pts=0
```

Current video/latency characteristics include:

- H.264 NVENC
- 1080p60
- approximately 20 Mbps CBR
- low-latency preset
- ultra-low-latency tuning
- zero-latency behavior
- no B frames
- GOP 30
- 1 MB-ish VBV target
- flush packets
- AAC 192 kbps / 48 kHz

Do not use:

```text
-repeat_headers 1
```

It is unsupported by the current h264_nvenc build.

Use:

```text
-bsf:v dump_extra=freq=keyframe
-g 30
```

This reduces cold-start PPS warnings but does not fully eliminate them.

Known warnings such as:

```text
non-existing PPS 0 referenced
```

remain deferred cleanup.

## Current SRT publisher

```text
srt://127.0.0.1:8890?streamid=publish:gameplay&pkt_size=1316&latency=20000&tlpktdrop=1
```

## Ingest experiments

### SRT localhost ingest

Current default.

Best overall behavior so far.

### RTSP/TCP ingest

Tested and rejected.

Pros:

- cleaner teardown
- PPS warning behavior improved

Cons:

- severe DirectShow real-time-buffer backpressure
- choppy/warped playback

### RTSP/UDP ingest

Tested and rejected.

Observed:

- RTP packet loss
- invalid FU-A errors
- choppy playback

Do not casually revisit RTSP ingest without a fundamentally different solution.

## MediaMTX

Current version:

```text
1.21.1
```

Current relevant config:

```yaml
readTimeout: 2s
writeTimeout: 10s
```

Transitional PowerShell bridge also uses:

```yaml
runOnDemandRestart: true
runOnDemandStartTimeout: 15s
runOnDemandCloseAfter: 5s
```

The earlier `readTimeout: 10s` allowed stale SRT publisher state to persist too long after shutdown.

Reducing it to `2s` improved rapid reconnect behavior.

## MediaMTX 1.21.1 source-verified demand behavior

During recovery debugging, MediaMTX 1.21.1 source was inspected directly.

Verified:

- Windows external commands run inside a kill-on-close Job Object.
- Stopping runOnDemand kills the command tree.
- Killing MediaMTX closes the Job Object and kills its command tree.
- Therefore orphaned PowerShell start scripts after a normal MediaMTX kill should not be assumed.
- `runOnDemandRestart: true` uses a fixed approximately five-second restart pause.
- Publisher loss does not reset the current on-demand state.
- Removing the final active reader arms close-after.
- A DESCRIBE waiting for a source is not counted as an active reader.
- A recovering Client may therefore be waiting while MediaMTX considers no reader active.
- The close-after timer can stop runOnDemand while that DESCRIBE is still waiting.
- `runOnDemandStartTimeout` applies to a fresh command-start cycle and rejects waiting requests when it expires.

This mismatch is architectural.

Do not attempt to solve it solely with sleep/timeout tuning.

## Transitional PowerShell bridge

The legacy/current bridge is:

```text
MediaMTX
-> runOnDemand
-> start-gameplay.ps1
-> FFmpeg
```

and:

```text
runOnUnDemand
-> stop-gameplay.ps1
```

Diagnostics added during recovery work include:

- PowerShell PID
- FFmpeg PID
- FFmpeg process snapshots
- previous PID-file value
- launch time
- exit code
- stop branch/reason
- lifecycle log file

`stop-gameplay.ps1` was hardened to:

- verify exact FFmpeg process
- request termination
- wait up to five seconds
- remove PID file only after confirmed exit
- retain PID state and warn if termination times out

This bridge is temporary.

Once native Host FFmpeg ownership is validated, do not continue investing in PID-file/process-script architecture except for rollback/reference cleanup.

## Native Host FFmpeg ownership

Approved target:

- Rust Host directly launches FFmpeg.
- Host retains child/process handle.
- Host owns intentional stop.
- Host owns restart.
- Host owns shutdown cleanup.
- Host guarantees at most one FFmpeg.
- PID file is not primary ownership.
- MediaMTX remains relay/demand infrastructure.

Product requirement:

When no viewers exist:

- no FFmpeg
- capture device unopened
- NVENC idle
- no publisher bandwidth

Do not simplify the architecture into an always-on encoder merely to avoid lifecycle complexity.

## Host singleton

Mutex:

```text
Global\PortableGameCasterHost.v1
```

Rules:

- acquire before MediaMTX, mDNS, Ctrl+C/resource ownership
- second Host exits before starting resources
- `ERROR_ACCESS_DENIED` can represent an already-running Host across privilege contexts
- handle is RAII-owned
- `.v1` is not Host SemVer

Multiple LAN Hosts are valid.

## mDNS

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

Windows advertiser uses `mdns-sd`.

`enable_addr_auto()` successfully supports IPv4 resolution on macOS.

Future TXT should use explicit protocol-version naming.

## macOS ffplay

Current Homebrew dependency:

```text
ffmpeg-full
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

Notes:

- `-sync ext` fixed playback pacing.
- larger probe/analyze values were needed for reliable AAC/rate detection.
- near-zero buffering removed old manual pause/seek workarounds.
- software decode performs adequately on M4.
- VideoToolbox experiment hit Vulkan portability issues.

## Client media liveness

Current health source is ffplay stderr/status output.

Example:

```text
5.58 A-V: -0.091 fd=   0 aq=    0KB vq=   29KB sq=    0B
```

Important:

With `-sync ext`, the first/master-clock column continues increasing during a frozen stream.

Therefore it is not a valid liveness signal.

Current algorithm:

1. MediaStarted confirmed.
2. Strip/ignore master-clock column.
3. Compare remaining status fields.
4. Healthy playback changes frequently.
5. Approximately five seconds of unchanged remainder emits `TransportEvent::Stalled`.
6. Client exits Playing and enters normal recovery.

Known limitations:

- if ffplay stops outputting status entirely, no new comparison arrives
- audio progress may change A/V-related fields while video alone is frozen

Backlog:

- no-telemetry timeout
- separate video/audio progress

Do not expand the interim parser prematurely without a real failure case.

## Client state rules

- UI state describes observable reality.
- Playing requires decoded media.
- Fresh connections never use reconnect states.
- Recovery remains pinned to the same Host.
- Restored RTSP enters `WaitingForStream`.
- Cancel immediately returns to Idle.
- Cancel invalidates worker generation.
- Cancel kills race-started ffplay.
- Stop Stream returns to Idle without recovery.
- Quit/window close terminates ffplay.
- No Client -> Host session-cancel message is desired.

Timeout copy:

```text
cold: Stream did not start.
recovery: Stream did not resume.
```

## Countdown implementation

Visible countdowns should use monotonic deadlines.

Current Client uses deadline-based timing for:

- reconnect
- warm-up fallback
- discovery polling cadence

Previous issue:

The worker calculated a displayed second, performed network/mDNS work, then emitted the stale value.

This made seconds visually uneven.

Do not reintroduce loop-cadence-derived countdowns.

## Logging

UTC/Zulu millisecond timestamps are deliberate.

Example:

```text
[07:23:18.492Z]
```

This makes multi-machine correlation direct.

### Current Client

- packaged app/no flag: Off
- development binary/no flag: Debug
- `--debug`: Debug
- `--verbose`: raw/Trace/Verbose
- `--quiet`: Off

### Current Host

Host does not yet have finalized Normal / Debug / Verbose parity.

Some diagnostic output remains unconditional.

MediaMTX output is currently relayed through the Host so Host-side UTC timestamps can be attached.

This removed MediaMTX's previous console colour because the process now sees a pipe instead of a terminal.

### Unified logging target

Normal:

- important lifecycle
- low overhead
- no raw external output

Debug:

- structured PGC diagnostics
- event-driven
- source label `[PGC]` may be omitted if redundant

Verbose:

- Debug plus raw external sources
- explicit source identity required:
  - PGC
  - FFPLAY
  - FFMPEG
  - MTX

Presentation:

- dim timestamp
- fixed-width columns
- stable colours
- same category colours across Host and Client
- aligned message column
- no ANSI in redirected output
- centralized formatting

## Host console presentation

Windows Host currently attempts to widen a classic attached console to approximately 140 columns.

Purpose:

FFmpeg progress output updates on one physical line rather than wrapping and visually spewing.

Behavior should remain best-effort.

Unsupported terminals or redirected output should not fail startup.

## Logging relay caveat

FFmpeg progress uses carriage-return updates.

When mixed with stamped newline-oriented logs:

- progress lines can visually collide with a newly stamped line
- stale trailing characters can remain
- progress may display one update late

The unified logging pass should preserve readable progress without building a large terminal-rendering framework.

## Audio routing prototype

Current Denon AVR-X2800H behavior:

Main HDMI audio output cannot simultaneously provide the desired AVR speaker path and capture-device HDMI audio path.

Prototype workaround:

```text
ZONE2 source
-> ZONE2 analog RCA
-> HD60 S+ analog input
```

This allows:

- normal AVR speaker playback
- stereo capture audio

Future GC575 lacks analog input; PC Line In or another capture-audio path may be used.

## Testing matrix

### Golden path

- Host starts
- Mac discovers Host
- stream requested
- media begins
- Client reaches Playing
- Stop Stream -> Idle
- Search works again
- Quit cleans player

### Client failures

- discovery unavailable
- RTSP unavailable
- stream starts slowly
- stream disappears
- ffplay killed
- media freezes without process exit
- recovery timeout

### Host failures

- FFmpeg killed
- MediaMTX killed
- Host killed
- rapid FFmpeg failure/restart
- rapid MediaMTX failure/restart
- awkward timing around demand start/stop

### Native Host ownership validation

Verify:

- Host idle contains no ffmpeg.exe
- first demand launches exactly one FFmpeg
- last demand eventually stops FFmpeg
- FFmpeg failure while demand exists causes Host restart
- MediaMTX failure does not leave stale FFmpeg
- repeated abuse never produces multiple owned FFmpeg instances
- Host shutdown leaves no MediaMTX or FFmpeg process
- PowerShell is absent from normal capture lifecycle