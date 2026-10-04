# Portable Game Caster Roadmap / Backlog

This backlog is ordered approximately by current priority, not by strict release commitment.

## Immediate client work
- Finish truthful Connected-state pass.
  - Connected only after actual media flow is confirmed.
  - Reconnecting only after a previously healthy Playing state is lost.
  - Transition Reconnecting -> WaitingForStream once host/RTSP handshake is restored.
  - Separate reconnect timeout from media warm-up timeout.
  - Current target: ~15 seconds for slower hosts/networks.
- Verify reconnect countdown reflects real elapsed time.
- Test repeated MediaMTX and FFmpeg kill/recovery cycles.
- Test full PGC Host process loss/restart while connected.
- Investigate intermittent manual ffplay close -> Search for Host -> incorrect reconnect-state path.
- Investigate rare immediate Search for Host mDNS miss after a stream is closed.

## Logging / diagnostics
- Centralized logging module with Off / Debug / Trace.
- Raw Cargo-built executable defaults to Debug.
- Packaged app defaults quiet.
- `--debug`, `--verbose`, `--quiet` runtime flags.
- Log every state transition.
- Log discovery hostname/IP/protocol/port/path/URL.
- Log ffplay path/PID/lifecycle.
- Debug-mode high-level stream summary:
  - video codec,
  - resolution,
  - frame rate,
  - audio codec,
  - sample rate,
  - channels,
  - bitrate only where reliable.
- Verbose mode keeps raw ffplay diagnostics.
- Future integrated in-app log panel/ring buffer.
- Future log-file output/export.

## Real media-flow health
- Detect whether video frames are actually arriving.
- Detect whether audio is actually arriving.
- Track last-media timestamp.
- Distinguish process alive / RTSP alive / media alive.
- Surface stalled-video, missing-audio, degraded-network conditions.
- Replace interim ffplay-log-derived MediaStarted signal with a first-class health model.

## Client stream UX
- Add connected-state stream information:
  - duration,
  - bitrate,
  - resolution,
  - frame rate,
  - useful network/connection health.
- Add `Synchronize` action while connected to return playback to the freshest safe live edge without audio warping/stuttering.
- Wrap ffplay in a dedicated PGC player window/app surface with custom title:
  - `Portable Game Caster Stream — <hostname>`
- Native macOS power assertion tied to active playback.

## Client discovery robustness
- Investigate rare immediate re-search misses.
- Test rapid Close Stream -> Search for Host cycles.
- Potentially retry/reuse mDNS browse once before surfacing failure.
- Preserve distinction between:
  - no service found,
  - service found but address unresolved.

## Dependency validation
Test and surface clear errors for missing/misconfigured prerequisites.

Host:
- MediaMTX missing.
- FFmpeg missing.
- invalid executable path.
- failed launch.
- unsupported FFmpeg/NVENC version.
- bad MediaMTX config.

Client:
- ffplay missing.
- Homebrew missing.
- ffmpeg-full missing.
- invalid `PGC_FFPLAY_PATH`.
- failed player launch.

Diagnostics should show what paths were searched and why startup failed.

## Windows host
- Replace PowerShell lifecycle with native Rust process ownership.
- Host owns:
  - MediaMTX,
  - FFmpeg,
  - discovery advertisement.
- Eliminate console/PowerShell flashes.
- Clean child-process ownership and shutdown.
- Investigate rare race where MediaMTX restarts but FFmpeg does not.
- Capture-device presence/health.
- Verify actual video/audio flow.
- Automatic recovery/nudging for flaky HD60 S+ behavior.
- Host status/health API consumed by clients.
- Install/run as a proper Windows background service/helper.

## Encoder/backend support
- Modern NVIDIA + current FFmpeg/NVENC.
- Pascal/legacy NVIDIA compatible FFmpeg profile.
- Intel QSV.
- AMD AMF.
- x264 fallback.
- Decide whether current + legacy FFmpeg bundles are practical/licensable.
- Treat hardware support as profile selection rather than bespoke host builds.

## Packaging / installation
Windows installer should eventually handle:
- PGC Host binary,
- MediaMTX,
- compatible FFmpeg,
- discovery/service setup,
- capture-device detection,
- firewall/configuration,
- background startup.

macOS client should eventually remove Homebrew as a hard dependency by bundling/managing the playback dependency where licensing/distribution permits.

## Latency
- High priority: reduce end-to-end latency further.
- Current MediaMTX relay adds roughly ~0.5 seconds vs direct point-to-point.
- Keep current stable transport until a clearly better architecture is proven.

## Known stream cleanup items
- SRT ACKACK log cleanup.
- Cold-start H.264 PPS warnings remain with SRT ingest; currently tolerated.
- Do not reintroduce RTSP ingest unless a fundamentally new solution addresses prior backpressure/choppiness issues.

## Recording / archival
- Archival recording mode, potentially concurrent with live stream.
- AVerMedia GC575 integration.
- Surround-format capture matrix.
- Evaluate AVR upgrades/HDMI audio behavior where relevant.

## Platform expansion
- Linux client/host support.
- Steam Deck.
- Windows client.
- macOS host where useful.
- Android.
- iOS/iPadOS.

Keep Client and Host product SemVer shared across platforms when feature parity is claimed.
