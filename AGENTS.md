# Portable Game Caster — Agent Guide

Portable Game Caster (PGC) is a free/open-source local-network console gameplay streaming/capture system under the JoeYoshi banner.

## Project identity

- License: `GPL-3.0-only`
- macOS bundle ID: `com.joeyoshi.portablegamecaster`
- Primary implementation language: Rust
- Repository: `joeyoshi/PortableGameCaster`
- Primary development branch: `master`
- Goal: portable, native-feeling, resilient, transparent software that is simple when healthy and explicit when something fails.

## Product principles

1. UI states describe observable reality, not implementation state.
2. `Connected` / `Playing` means real media flow has been confirmed, not merely that a process, port, or RTSP handshake exists.
3. Fresh/cold connection attempts must not use reconnect states. Reconnect states are only valid after a previously healthy Playing session is lost.
4. Reconnecting means restoring host/RTSP connectivity. Once the handshake is back, transition to `WaitingForStream` while media warms up.
5. Prefer explicit, debuggable failure states over silent fallback.
6. Idle Host footprint is a first-class product requirement.
7. When no Client demand exists:
   - FFmpeg should not be running,
   - the capture device should not be held open,
   - NVENC should be idle,
   - no gameplay publisher bandwidth should be produced.
8. A few seconds of first-stream startup/warm-up latency is acceptable in exchange for low idle resource usage and reliable lifecycle behavior.
9. Keep Normal-mode overhead low. Richer diagnostics belong behind Debug and Verbose logging modes.
10. Preserve platform-native behavior where practical; share core logic rather than forcing a lowest-common-denominator UI.
11. Prefer one portable app per role with runtime flags/settings over separate debug/release application variants.
12. PGC is a broadcast/listener system. Clients should not tightly own Host sessions.
13. MediaMTX is relay/service infrastructure. The native Host should own capture/encoder lifecycle.

## Agent working contract

Treat requests as scoped engineering tickets.

- Read the relevant repository docs before changing architecture-sensitive code.
- Inspect existing code before implementing.
- Prefer the smallest clean change that satisfies the ticket.
- Preserve unrelated local changes.
- Do not commit, push, merge, rebase, tag, or publish releases unless explicitly asked.
- Leave the working tree dirty for hands-on review and provide a suggested commit message.
- Architecture, product, and UX decisions are made by the user in collaboration with ChatGPT. Do not silently broaden scope or substitute a new product decision.
- A request for explanation or rationale is not automatically a request to change implementation. Only change behavior when the ticket or user explicitly asks for a change.
- If the user asks for options, present options rather than silently choosing and implementing one unless the ticket explicitly delegates that decision.
- Ask only when genuinely blocked by missing information or an unresolved architectural ambiguity.
- Engineering validation is not UX approval.
- Do not claim native target-platform validation from a different OS.
- If target and current environment differ, run only platform-neutral checks and state exactly what remains unvalidated.
- Do not install cross-target toolchains solely to create the appearance of target-platform validation.
- Assume no internet unless it is actually available. If external source behavior matters, state whether it was verified or inferred.
- Do not tune sleeps/timeouts merely to hide lifecycle races unless the ticket explicitly calls for a timing experiment and evidence supports it.

## Status vocabulary

Use these terms when applicable:

- `Implemented`
- `Engineering Validated`
- `Awaiting Hands-on Validation`
- `Awaiting Hands-on UX Validation`
- `UX Approved`
- `Done`

Do not collapse implementation, engineering validation, and UX approval into one status.

## Agent report format

Return the entire report in one fenced Markdown block so it can be copied back into the design/review conversation.

Use these sections:

- Implemented
- Behavior
- Engineering Validation
  - current environment
  - target
  - commands/tests run
  - result
  - explicitly not validated
- UX Review Notes
- Documentation Impact
- Notes
- Suggested Commit Message
- Status

## Documentation Impact

Every implementation report must explicitly assess:

- `AGENTS.md`
- `docs/PGC-ARCHITECTURE.md`
- `docs/PGC-ROADMAP.md`
- `docs/PGC-DEV-NOTES.md`
- `docs/PGC-DECISIONS.md`
- `docs/PGC-VERSIONING.md`

For each, say:

- `update needed`, with a brief suggested change; or
- `none`.

Do not automatically rewrite canonical docs during every coding ticket unless the ticket explicitly requests documentation changes.

### Canonical documentation workflow

The repository docs are shared project memory for the user, ChatGPT, and coding agents.

Use this flow:

1. Product/architecture decisions may be recorded in ROADMAP and DECISIONS immediately.
2. Experimental implementation does not automatically become architectural truth.
3. Claude/coding agents flag documentation impact in every report.
4. After hands-on validation/approval, synchronize ARCHITECTURE and DEV-NOTES to accepted behavior.
5. Prefer committing accepted code and its documentation sync together.
6. When a milestone is declared approved or ready to commit, check documentation before considering the checkpoint complete.

### Document responsibilities

- `AGENTS.md`
  - agent operating rules and workflow contract
- `PGC-ARCHITECTURE.md`
  - current accepted architecture plus clearly marked active transitions
- `PGC-ROADMAP.md`
  - prioritized future work and milestone status
- `PGC-DEV-NOTES.md`
  - implementation constraints, experiments, landmines, and platform-specific knowledge
- `PGC-DECISIONS.md`
  - durable product/architecture decisions and their rationale
- `PGC-VERSIONING.md`
  - product/protocol/build version policy

## Logging modes

Client already supports runtime logging modes. Host parity is a near-term roadmap item.

Shared target semantics:

### Normal

- important lifecycle/user-facing events only
- low overhead
- no raw external-process firehose
- suitable as the default production mode

### Debug

- everything useful for structured PGC diagnostics
- state/lifecycle reasoning
- discovery and connection decisions
- health/recovery events
- process ownership events
- event-driven, not a raw per-frame/per-packet feed
- should remain practical for troubleshooting without major performance cost

### Verbose

- Debug plus raw external-source output
- raw ffplay / FFmpeg / MediaMTX output where useful
- higher console I/O and formatting overhead is acceptable
- every line must make its source unambiguous

## Logging presentation direction

Shared target layout:

```text
Debug:
[07:50:41.098Z] [STATE]      Playing("carock-pgc.local") | Connected.
[07:50:52.209Z] [HEALTH]     Active stream transport lost

Verbose:
[07:50:41.098Z] [PGC]    [STATE]      Playing("carock-pgc.local") | Connected.
[07:50:54.335Z] [FFPLAY]             Input #0, rtsp, from 'rtsp://...'
[07:50:54.336Z] [MTX]                ...
[07:50:54.337Z] [FFMPEG]             ...
```

Rules:

- timestamps use UTC/Zulu with millisecond precision
- timestamp is dim/grey in interactive terminals
- source and category columns are fixed-width
- message text begins at one consistent column
- category colours are stable and consistent
- categories shared by Host and Client should use the same colour
- Debug may omit redundant `[PGC]` source identity when all visible lines are PGC-native
- Verbose requires explicit source identity on every line
- raw/external output must never be ambiguous about its source
- terminal colour is presentation-only
- redirected/file output should remain clean plain text unless colour is explicitly forced
- centralize formatting/styling rather than scattering ANSI codes through call sites

Potential PGC categories include:

- `APP`
- `STATE`
- `CONNECT`
- `DISCOVERY`
- `PLAYER`
- `STREAM`
- `HEALTH`
- `RECONNECT`
- `HOST`
- `CAPTURE`
- `ENCODER`
- `DEMAND`

## Current macOS logging behavior

- packaged app with no flags: Off
- raw Cargo-built executable with no flags: Debug
- `--debug`: Debug
- `--verbose`: Trace/Verbose plus raw ffplay diagnostics
- `--quiet`: Off

## Current Client states

- `Idle`
- `Discovering`
- `Resolving(host)`
- `Connecting(host)`
- `WaitingForStream(host)`
- `Playing(host)`
- `ReconnectingStream { host, seconds_remaining }`
- `ReconnectingHost { host, seconds_remaining }`
- `Error(message)`

## Client session-control behavior

- Idle: `Search for Host`
- Busy / warm-up / reconnecting: `Cancel`
- Playing: `Stop Stream`
- Error: `Retry`
- `Quit` remains available

Rules:

- Cancel must return immediately to Idle.
- Cancel invalidates stale worker messages.
- Cancel terminates any race-started ffplay process.
- Stop Stream terminates ffplay and returns to Idle without entering recovery.
- Do not add Client -> Host cancel/session coupling.
- PGC remains a broadcast/listener architecture.

## Current stream topology

Accepted media topology:

```text
Console / AVR / capture device
-> Windows FFmpeg
-> SRT localhost
-> MediaMTX
-> RTSP/TCP LAN
-> macOS Client / ffplay
-> Discord
```

Host-side FFmpeg ownership is actively transitioning from MediaMTX/PowerShell `runOnDemand` ownership to native Rust Host ownership.

Until that work is hands-on validated:

- PowerShell/runOnDemand remains the current implementation.
- Native Host ownership is the approved target architecture.

## Windows Host lifecycle direction

Approved target:

```text
PGC Host
|- MediaMTX supervisor
|- native FFmpeg owner
|- demand-state interpretation
|- mDNS advertisement
|- recovery/lifecycle logic
`- future configuration / capture portability
```

The Host should:

- start idle with no FFmpeg
- detect demand
- launch FFmpeg directly
- retain the FFmpeg process handle
- guarantee at most one owned FFmpeg
- monitor expected/unexpected exit
- restart FFmpeg while demand remains
- stop FFmpeg when demand ends
- cleanly stop FFmpeg and MediaMTX on Host shutdown
- avoid PID files as the primary ownership mechanism

## Known Windows media constraints

- Current FFmpeg: Gyan 8.0.1 full build.
- Do not casually upgrade it.
- Newer tested builds require NVENC API/driver support unavailable on the current GTX 1080 Ti system.
- DirectShow video:
  - `Game Capture HD60 S+`
- DirectShow audio:
  - `Digital Audio Interface (Game Capture HD60 S+)`
- Capture is YUY2 1920x1080@60 and must be converted to NV12 before Pascal NVENC.
- Do not use `-use_wallclock_as_timestamps 1`.
- `-repeat_headers 1` is unsupported in the current h264_nvenc build.
- `-bsf:v dump_extra=freq=keyframe` plus `-g 30` reduces but does not eliminate cold-start PPS warnings.
- FFmpeg publishes SRT locally to MediaMTX.
- RTSP ingest was tested and rejected due backpressure/choppiness and packet-loss behavior.

## MediaMTX lifecycle notes

Current MediaMTX version: `1.21.1`.

Current path:

```text
/gameplay
```

SRT publisher:

```text
localhost:8890
streamid=publish:gameplay
```

RTSP reader:

```text
port 8554
/gameplay
```

Current transitional bridge includes:

- `readTimeout: 2s`
- `writeTimeout: 10s`
- `runOnDemandRestart: true`
- `runOnDemandStartTimeout: 15s`
- `runOnDemandCloseAfter: 5s`

Verified from MediaMTX 1.21.1 source:

- waiting DESCRIBE requests are not counted as active readers
- publisher loss can leave a Client still attempting recovery while MediaMTX considers no reader active
- runOnDemand restart uses a fixed ~5 second restart pause
- this can collide with the current 5 second close-after timing
- Windows external commands are owned by a kill-on-close Job Object
- stopping/killing MediaMTX normally terminates its PowerShell/FFmpeg runOnDemand command tree

These semantics are a primary reason to remove FFmpeg lifecycle ownership from MediaMTX.

## Windows Host singleton

Only one PGC Host should run per Windows machine.

Current mutex:

```text
Global\PortableGameCasterHost.v1
```

Rules:

- mutex is acquired before other Host-owned resources
- second launch exits cleanly
- `ERROR_ACCESS_DENIED` may be treated as an existing instance across privilege/account contexts
- mutex name represents the invariant one-Host-per-machine contract
- do not casually rev `.v1` with application SemVer
- multiple PGC Hosts on one LAN remain valid

## macOS player notes

Known-good ffplay options include:

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

Software decode is currently preferred. VideoToolbox experiments hit Vulkan portability issues and are not needed on M4.

## Client media truthfulness

- MediaStarted is derived from ffplay telemetry.
- Process existence alone is not health.
- RTSP connection alone is not health.
- Playing requires decoded media.
- After playback begins, the Client monitors ffplay progress.
- The `-sync ext` master clock is not a valid liveness signal because it continues advancing during a media stall.
- About five seconds of unchanged non-master-clock status data triggers a stall/recovery event.

Known follow-ups:

- detect ffplay itself ceasing to emit status entirely
- distinguish video progress from audio progress so audio cannot mask a frozen video stream

Do not over-engineer these until scheduled.

## Discovery protocol

Service:

```text
_pgc._tcp.local.
```

Current TXT data:

```text
protocol=rtsp
path=/gameplay
version=1
```

Protocol compatibility must remain independent from Host and Client SemVer.

Future metadata should use an explicitly named protocol-version field.

## Versioning policy

Use three independent concepts:

1. PGC Client SemVer
2. PGC Host SemVer
3. PGC protocol version

Platform-specific build numbers may differ while sharing product SemVer when feature parity is intact.

## Rust / architecture conventions

- Keep AppKit UI work on the main thread.
- Discovery/network/player/reconnect work belongs on worker threads.
- Communicate UI state changes through channels.
- Keep child-process ownership explicit.
- Prefer responsibility-based modules over large monolithic files.
- Keep reconnect logic separate from UI rendering.
- Preserve exhaustive state handling.
- Explicit user actions that change state should eventually have concise Debug breadcrumbs.

## Current macOS module direction

- `discovery.rs`
  - mDNS discovery and same-host rediscovery
- `player.rs`
  - ffplay location, launch, stderr/health parsing, transport events, stream metadata
- `state.rs`
  - user-facing state model
- `logging.rs`
  - runtime logging configuration and formatting
- `ui.rs` / future `ui/`
  - AppKit lifecycle and rendering
- `worker/mod.rs`
  - high-level stream lifecycle
- `worker/reconnect.rs`
  - same-host reconnect/handshake recovery
- `worker/countdown.rs`
  - monotonic deadline-driven countdown ticker

## Do not regress

- Closing ffplay manually should return the Client to Idle with Search available.
- Quit/window close should terminate ffplay.
- Cancel and Stop Stream must not accidentally enter recovery.
- Recovery must not silently jump to a different PGC Host.
- Discovery must tolerate `ServiceResolved` before IPv4 address resolution.
- Normal app UX must remain usable without a terminal.
- Playing must never remain indefinitely true solely because ffplay/RTSP stayed open after media froze.
- Idle Host behavior must not regress into an always-on capture/encode stream.