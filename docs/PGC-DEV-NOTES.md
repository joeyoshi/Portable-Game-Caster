# Portable Game Caster Development Notes

Implementation knowledge, tested constraints, failed experiments, and technical landmines that should not need to be rediscovered.

## Windows FFmpeg / capture constraints

Current prototype Host uses **Gyan FFmpeg 8.0.1 full**. Do not casually upgrade it: newer tested builds required NVENC API 13.1 / driver >= 610 while the current GTX 1080 Ti / Pascal system exposes API 13.0.

Current DirectShow inputs:

```text
video: Game Capture HD60 S+
audio: Digital Audio Interface (Game Capture HD60 S+)
```

Observed input:

- YUY2 / yuyv422, 1920x1080 @ 60 fps
- PCM s16le, 44.1 kHz, stereo

Pascal NVENC cannot directly ingest the current YUY2/4:2:2 input. Convert to NV12 first.

Current encode characteristics to preserve unless intentionally retuning:

- H.264 NVENC, 1080p60
- about 20 Mbps CBR
- low-latency preset / ULL tuning / zerolatency
- no B frames
- GOP 30
- AAC 192 kbps / 48 kHz
- `aresample=48000:async=1000:first_pts=0`
- `-flush_packets 1`
- `-bsf:v dump_extra=freq=keyframe`

Do **not** use `-use_wallclock_as_timestamps 1`; it caused non-monotonic DTS. Do **not** suggest `-repeat_headers 1`; unsupported by this h264_nvenc build.

Current SRT publisher:

```text
srt://127.0.0.1:8890?streamid=publish:gameplay&pkt_size=1316&latency=20000&tlpktdrop=1
```

## Ingest experiments

SRT localhost ingest is the current default and best overall tested behavior.

RTSP/TCP ingest was tested and rejected because DirectShow real-time-buffer backpressure produced choppy/warped playback. RTSP/UDP ingest produced packet loss / invalid FU-A behavior and choppy playback.

Do not casually revisit RTSP ingest without a fundamentally different solution.

## MediaMTX

Current version: `1.21.1`.

```yaml
readTimeout: 2s
writeTimeout: 10s
runOnDemand: $PGC_HOST_EXE --demand-signal
runOnDemandRestart: false
runOnDemandStartTimeout: 15s
runOnDemandCloseAfter: 10s
```

No active `runOnUnDemand`.

Important source-verified behavior:

- DESCRIBE waiting for a source is not an active reader
- removing the final active reader arms close-after
- a recovering Client may still be waiting while MediaMTX considers no reader active
- `runOnDemandStartTimeout` applies to a fresh command-start cycle

Therefore MediaMTX is not authoritative for FFmpeg recovery/lifecycle.

## Native Host demand signaling

Primary implementation: `windows/service/src/demand.rs`.

MediaMTX launches `pgc-host-windows.exe --demand-signal`. Helper mode runs before normal Host singleton/console startup, connects to the running Host over localhost, sends the expected hello, then holds the connection open. Open connection equals one demand signal.

The Host demand listener binds localhost on an ephemeral port, ignores connections without the expected hello, and can reset/drop demand signals when MediaMTX exits.

This intentionally allows arbitrary compatible RTSP readers to trigger demand without PGC-specific Client control messages.

## Native Host FFmpeg ownership

Primary implementation: `windows/service/src/ffmpeg.rs`.

FFmpeg discovery order:

1. `PGC_FFMPEG_PATH`
2. `C:\ffmpeg\bin\ffmpeg.exe`
3. `ffmpeg\ffmpeg.exe` beside Host

Missing FFmpeg currently fails Host startup.

Lifecycle:

- no demand -> no FFmpeg
- first demand -> launch FFmpeg
- at most one owned encoder
- unexpected encoder exit while demand remains -> restart with 1s / 2s / 5s backoff
- backoff resets after about 10s healthy
- no demand -> 5s Host grace, then send `q`, wait ~3s, escalate if necessary, confirm up to ~5s
- MediaMTX failure -> clear demand, stop/confirm FFmpeg, restart MediaMTX
- Host shutdown -> confirm FFmpeg then MediaMTX are gone

If a process refuses termination, keep ownership and do not launch a replacement on top of it.

## Job Object containment

`windows/service/src/job.rs` creates a kill-on-close Job Object and assigns MediaMTX/FFmpeg best-effort. Purpose: abnormal Host termination should not orphan infrastructure that holds ports/capture/encoder resources.

## Legacy PowerShell bridge

Old `runOnDemand -> start-gameplay.ps1 -> FFmpeg` / PID-file ownership is deprecated. Scripts may remain for archaeological/reference value, but active lifecycle must not silently drift back to them.

## Host singleton

```text
Global\PortableGameCasterHost.v1
```

Acquire before Host-owned resources. `ERROR_ACCESS_DENIED` may represent an existing instance across privilege contexts. `.v1` is not SemVer.

## Native Windows baseline

Previous real Windows validation of native Host ownership confirmed:

- idle start with no FFmpeg
- real HD60 S+ capture/NVENC on demand
- FFmpeg unexpected-exit restart
- MediaMTX unexpected-exit cleanup/restart
- graceful `q` stop
- no active PowerShell lifecycle
- lightweight idle recovery

Informal resource baseline:

```text
idle:   ~35 MB, effectively no CPU/GPU
active: ~240 MB, ~11% GPU
```

## First-stream slow-reader / audio-warble observation

One first real connection after the native ownership change showed warbly audio and MediaMTX `reader is too slow, discarding 1071 frames`; immediate later sessions were healthier.

No root cause established. Possible correlation areas: first capture-device open, DirectShow/card warm-up, initial clocks, MediaMTX burst/buffering, ffplay joining behind live edge. Do not retune transport/buffers from this single observation.

## Windows console / hotkeys transition

Current logging branch introduces `hotkeys.rs` and console `L` to open logs. Native Windows validation must verify Ctrl+C, redirected-input behavior, repeated `L`, and shutdown.

Known must-fix defect: classic Windows QuickEdit/selection can pause apparent console progress after the user clicks/selects text. Disable selection-induced suspension programmatically while preserving required input flags and re-test `L` and Ctrl+C.

Future Host console direction is deliberately lightweight: logs scroll normally, with a restrained footer/hotkey/status region rather than a heavyweight full-screen TUI. Likely future controls include `S` Settings and `H` Help, but those are not yet final.

## macOS ffplay

Current Homebrew dependency: `ffmpeg-full`.

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

Software decode performs adequately on M4. VideoToolbox experiments hit Vulkan portability issues and are not needed currently.

## Client media liveness

ffplay status telemetry is the current health source. With `-sync ext`, the first/master-clock value keeps advancing during a frozen stream and is not a liveness signal.

Current algorithm after MediaStarted:

1. ignore master clock;
2. compare remaining status fields;
3. about five seconds unchanged -> `TransportEvent::Stalled`;
4. terminate player and enter normal recovery.

Known gaps: ffplay may stop emitting status entirely; audio-related fields may move while video is frozen.

## Client state/control rules

- UI describes observable reality.
- Playing requires decoded media.
- Fresh connection never uses reconnect states.
- Recovery stays pinned to same Host.
- Restored RTSP -> WaitingForStream.
- Cancel immediately returns Idle and invalidates stale workers.
- Stop Stream returns Idle without recovery.
- Quit/window close terminates player.
- No Client -> Host session-cancel message.

Timeout copy:

```text
cold:     Stream did not start.
recovery: Stream did not resume.
```

Visible countdowns use monotonic deadlines. Do not reintroduce loop-cadence-derived countdowns.

## Unified logging implementation

UTC/Zulu millisecond timestamps are deliberate for cross-machine correlation.

Conceptual levels: Quiet / Normal / Debug / Verbose.

Current Mac and Windows logging cores are intentionally byte-identical through the shared core region. Do not let them drift casually. A shared crate is a pre-release architecture-audit candidate, not an automatic refactor just because duplication exists.

### Session files

File level policy:

```text
terminal Quiet/Normal/Debug -> file Debug
terminal Verbose            -> file Verbose
```

Mac folder:

```text
~/Library/Logs/Portable Game Caster/
```

Accepted Mac naming:

```text
pgc-client-latest.log
pgc-client-YYYY-MM-DDTHH-MM-SS.sssZ.log
```

At launch, archive previous latest using the session ID embedded in its header. If session ID is unreadable, fallback to mtime. If rename cannot be completed safely, append/warn rather than truncating. Only exact-pattern archives rotate; keep newest five. Renamed/copied/unrelated logs are never touched.

Windows Host currently uses timestamped per-launch logs and has not yet aligned to `latest`.

`PGC_LOG_DIR` is the test/override folder.

### FFmpeg output diagnostics

`encoder_output.rs` keeps a small recent FFmpeg stderr window (currently 12 useful lines, 300-char bound), filters banner noise, extracts likely causes, and supports publisher-wait diagnostics. Raw subprocess output remains Verbose.

### Synchronous logging

Current sinks write synchronously. Do not prematurely optimize. During the future shared-core audit, consider:

```text
producers -> bounded event queue -> one logging worker -> terminal/file/UI sinks
```

No thread per message, no unbounded queue. Critical structured events should be preserved; low-value raw Verbose chatter may eventually be throttled/dropped if a logger falls behind rather than disturbing media/supervision.

### Log-file deletion edge case

If a user manually deletes active `pgc-client-latest.log` while the app is open, the Unix file handle may remain valid while the path disappears. Acknowledge as low-priority hardening; do not solve until it matters.

## macOS interaction implementation notes

- utility shelf child controls must actually be parented to the shelf; adding a view to a second parent reparents it
- hover implementation uses a restrained overlay because a bezel tint did not render in off-screen checks
- the current overlay assumes a regular push-button bezel around 24pt high; revisit during architecture/UX refactor if platform variation matters
- initial focus is none
- focus transfers within the logical primary-action slot
- no default button
- window-level key policy owns `L`, Space/Return/Enter repeat suppression, and focused Return/Enter activation
- editable text and Cmd/Ctrl/Option combinations are not intercepted
- one physical activation should result in one action even if the key is held

Development-only `--ui-self-test` sends synthetic events through the real window path without discovery. Optional `PGC_UI_SNAPSHOT_DIR` renders snapshots. Release binary does not contain the self-test.

## Cross-machine handoff notes

Patch/snapshot handoffs must include untracked files and a declared baseline. Full-file snapshots are a valid fallback when line-ending conversion makes patches brittle. Keep the source working copy until the target reconstructs and verifies the delta.

Do not treat a Mac build/test of Windows-guarded code as native Windows validation.

## Testing priorities

Golden path:

- Host idle
- Client discovers/connects
- demand starts encoder
- decoded media -> Playing
- Stop -> Idle
- Search again
- Quit cleanly

Windows regression matrix should include:

- repeated console clicks/selection without Host pause
- `L` repeated
- Ctrl+C first press
- FFmpeg kill/restart
- MediaMTX kill/reset/restart
- no-demand teardown
- hard-kill orphan containment
- session log creation/rotation
- Normal/Debug/Verbose/Quiet presentation

macOS interaction regression matrix should include:

- utility shelf parenting
- hover / pressed / focus distinction
- `L` repeated and Caps Lock behavior
- Space/Return/Enter single activation
- held-key suppression
- Search -> Cancel -> Stop -> Search focus continuity
- no accidental Quit/default-button behavior
