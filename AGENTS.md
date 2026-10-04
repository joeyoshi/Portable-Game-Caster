# Portable Game Caster — Agent Guide

Portable Game Caster (PGC) is a free/open-source local-network console gameplay streaming/capture system under the JoeYoshi banner.

## Project identity
- License: GPL-3.0-only
- macOS bundle ID: `com.joeyoshi.portablegamecaster`
- Primary implementation language: Rust
- Goal: portable, native-feeling, resilient, transparent software that is simple when healthy and explicit when something fails.

## Product principles
1. UI states describe observable reality, not implementation state.
2. `Connected` means real media flow has been confirmed, not merely that a process/port exists.
3. Fresh/cold connection attempts must not use reconnect states; reconnect states are only valid after a previously healthy Playing session is lost.
4. Reconnecting means restoring host/RTSP connectivity. Once the handshake is back, transition to `WaitingForStream` while media warms up.
5. Prefer explicit, debuggable failure states over silent fallback.
6. Keep normal-mode overhead low; diagnostics belong behind runtime logging levels.
7. Preserve platform-native behavior where practical; share core logic rather than forcing a lowest-common-denominator UI.
8. Prefer one portable app per role with runtime flags over separate debug/release app variants.

## Logging modes
- Packaged app with no flags: logging Off.
- Raw Cargo-built executable with no flags: Debug logging by default.
- `--debug`: Debug logging.
- `--verbose`: Trace logging and raw ffplay diagnostic passthrough.
- `--quiet`: Off.

Logging should be centralized and state transitions should be logged whenever they are sent to the UI.

## Current client states
- `Idle`
- `Discovering`
- `Resolving(host)`
- `Connecting(host)`
- `WaitingForStream(host)`
- `Playing(host)`
- `ReconnectingStream { host, seconds_remaining }`
- `ReconnectingHost { host, seconds_remaining }`
- `Error(message)`

## Current stream topology
`Console / AVR / capture device -> Windows FFmpeg -> SRT localhost -> MediaMTX -> RTSP/TCP LAN -> macOS client / ffplay -> Discord`

## Known Windows media constraints
- Current Windows FFmpeg: Gyan 8.0.1 full build.
- Do not casually upgrade: newer builds require newer NVENC API/driver than the GTX 1080 Ti system currently exposes.
- DirectShow video: `Game Capture HD60 S+`
- DirectShow audio: `Digital Audio Interface (Game Capture HD60 S+)`
- Capture is YUY2 1920x1080@60 and must be converted to NV12 before Pascal NVENC.
- Do not use `-use_wallclock_as_timestamps 1`.
- `-repeat_headers 1` is unsupported in the current h264_nvenc build; do not reintroduce it.
- `-bsf:v dump_extra=freq=keyframe` and `-g 30` reduce, but do not eliminate, cold-start PPS warnings.
- FFmpeg publishes SRT locally to MediaMTX; RTSP ingest was tested and rejected due backpressure/choppiness and packet-loss behavior.

## MediaMTX lifecycle notes
- Current host uses MediaMTX 1.21.1.
- Path: `/gameplay`
- SRT publish: localhost:8890, streamid `publish:gameplay`
- RTSP client path: port 8554 `/gameplay`
- `readTimeout: 2s`
- `runOnDemandCloseAfter: 5s`
- Existing PowerShell start/stop scripts own FFmpeg today; replacing them with native host process ownership is a major roadmap item.

## macOS player notes
Known-good ffplay options include:
- `-rtsp_transport tcp`
- `-fflags nobuffer`
- `-flags low_delay`
- `-noinfbuf`
- `-framedrop`
- `-sync ext`
- `-probesize 2M`
- `-analyzeduration 500000`
- `-max_delay 0`
- `-stats`

Software decode is currently preferred; VideoToolbox experiments hit Vulkan portability issues and are not needed on M4.

## Discovery protocol
mDNS service type: `_pgc._tcp.local.`

Current TXT data:
- `protocol=rtsp`
- `path=/gameplay`
- `version=1`

The version field should evolve into an explicitly named protocol version. Protocol compatibility must remain independent from Host and Client SemVer.

## Versioning policy
Use three independent concepts:
1. PGC Client SemVer shared across client platforms.
2. PGC Host SemVer shared across host platforms.
3. PGC protocol version independent of both.

Platform-specific build numbers may differ while sharing the same product SemVer when feature parity is intact.

## Rust / architecture conventions
- Keep AppKit UI work on the main thread.
- Discovery/network/player/reconnect work belongs on worker threads.
- Communicate UI state changes through channels.
- Keep ffplay ownership explicit through a shared child-process handle.
- Prefer responsibility-based modules over large monolithic files.
- Keep reconnect logic separate from UI rendering.
- Preserve exhaustive state handling.

## Current macOS module direction
- `discovery.rs`: mDNS discovery and same-host rediscovery.
- `player.rs`: ffplay location, launch, stderr/health parsing, transport events, stream metadata.
- `state.rs`: user-facing state model.
- `logging.rs`: runtime logging configuration and log formatting.
- `ui.rs` / future `ui/`: AppKit lifecycle and rendering.
- `worker/mod.rs`: high-level stream lifecycle.
- `worker/reconnect.rs`: same-host reconnect/handshake recovery.

## Do not regress
- Closing ffplay manually should return the client to Idle with `Search for Host` available.
- Quit/window close should terminate ffplay so MediaMTX no longer sees an active reader.
- Reconnect recovery must not silently jump to a different PGC host.
- Discovery must tolerate a `ServiceResolved` event arriving before an IPv4 address record.
- Normal app UX must remain usable without a terminal.
