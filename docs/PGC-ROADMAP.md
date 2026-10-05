# Portable Game Caster Roadmap / Backlog

Ordered approximately by current priority, not strict release commitment.

Statuses:

- `Planned`
- `In Progress`
- `Awaiting Hands-on Validation`
- `Awaiting Hands-on UX Validation`
- `UX Approved`
- `Done`

Planned/future sections describe intent, not current product behavior.

---

## Immediate — unified Host + Client logging

**Status: In Progress — macOS Client UX Approved; Windows Host awaiting native validation. Feature not Done.**

Accepted macOS baseline:

- Quiet / Normal / Debug / Verbose
- UTC/Zulu millisecond timestamps
- readable Normal/Debug STATE lines; raw internal state only Verbose
- 50-character startup header with build channel + SemVer, platform, mode, protocol, session, logs folder
- plain session files independent of terminal mode
- active `pgc-client-latest.log`; millisecond archive naming; newest five exact-pattern archives
- action/final-state breadcrumbs
- top-right utility shelf + Open Logs Folder (`L`)
- repeat-safe keyboard handling, no default button, focus follows logical primary slot
- hover/pressed feedback
- development-only UI self-test/snapshot support

Windows Host implementation awaiting native acceptance includes:

- same conceptual levels/presentation/sinks
- `[MTX]` / `[FFMPEG]` raw output at Verbose
- Normal DEMAND / ENCODER / STREAM lifecycle
- publisher wait/availability and configured media
- wait warnings at 5 / 15 / 30 seconds
- unexpected encoder `Cause:` extraction
- truthful demand-loss wording
- `L` opens logs folder

Remaining after Windows validation:

- decide whether Host should adopt Client-style `latest` naming
- review provisional Host category colors on real console
- reconcile final Windows findings before merge to `master`

---

## Immediate — Windows Host console QuickEdit / click-freeze

**Status: Planned as part of native Windows validation**

Observed: clicking/selecting in the classic Windows console can pause apparent Host progress until the selection is released, making healthy functionality look broken.

Required:

- disable selection-induced suspension programmatically
- preserve Ctrl+C and `L`
- degrade safely when no interactive console exists
- validate idle and streaming while clicking, dragging, double-clicking, and selecting

---

## Immediate — Windows Host portability / capture configuration

**Status: Planned — one of the next major milestones**

Remove prototype capture hardcoding.

Initial target:

- enumerate Windows video sources
- enumerate audio sources independently
- first-run arrow-key selection + Enter confirmation
- save only after full video+audio confirmation
- if configured hardware disappears, re-enter configuration while preserving still-valid choices where sensible
- arbitrary DirectShow capture devices first
- external capture first
- portable configuration model that can later support other backends

Do **not** put bitrate, frame rate, or resolution into initial capture-source configuration. Those belong to later stream-profile/negotiation work.

Likely idle Host shortcut: `S` Settings. Not final until implemented/validated.

---

## Immediate / observe — first-stream lag / audio warble

**Status: Planned / reproduce before changing transport**

One first connection showed distorted/warbly audio and MediaMTX `reader is too slow` with 1,071 discarded frames. Immediate later streams were healthier.

Investigate correlation with first capture-device open, DirectShow/card warm-up, initial clocks, MediaMTX buffering/burst, ffplay joining behind live edge. Do not retune buffering/timing without reproducible evidence.

---

## Client recovery / truthfulness

**Status: Core UX Approved; soak/health improvements remain**

Accepted:

- Playing only after decoded media
- reconnect states only after previously healthy Playing
- restored RTSP -> WaitingForStream
- Cancel / Stop / Retry contextual controls
- monotonic reconnect/warm-up/discovery countdowns
- same-Host recovery
- initial frozen-media watchdog

Remaining:

- long healthy soak for false positives
- no-telemetry watchdog
- separate video/audio liveness
- first-class media-health model

---

## Planned first-release — Client Host Browser

Current auto-connect behavior should evolve before first release.

Direction:

- launch into discovery/home screen
- discover Hosts but do **not** auto-connect
- user explicitly selects a Host
- support multiple Hosts naturally
- Automatic Host Discovery default ON
- prefer event/service add/remove + expiry rather than aggressive polling
- when auto discovery is off, explicit Search/Refresh
- direct-IP escape hatch

Row direction:

```text
CAROCK                               192.168.0.242
                         Ready
```

Thin full-width clickable row, no separate Connect button. Potential color-coded statuses: Ready, Busy/Streaming, Updating, Capture Source Missing, Incompatible, Unavailable. Exact status/protocol model remains future design.

Direct-IP utility icon likely sits in the top-right utility shelf, left of Logs, on Host Browser only.

### Host Browser tooltip/details

Hover information should be operationally useful:

```text
Host
Address
Version
Protocol
Video
Audio

Status
status-specific message

Connection: Excellent (20 ms)   # only once real metrics exist
```

Use `Connection` or `Network`, not "Connection Strength"; PGC is not measuring RF signal strength.

Future connection quality may consider RTT, jitter, loss, reconnect stability, etc. Do not claim metrics that do not exist.

Tooltip should be able to update while open. A dim hint such as `See More Detailed Host Info — Hold Option` may reveal an info affordance/dense Host Info modal.

A reusable custom tooltip/popover system is backlog because native tooltip timing/content is too limited for live rich Host Browser information.

---

## Planned first-release — native macOS menu

Application menu direction:

- About Portable Game Caster
- Check for Updates…
- Settings… later
- standard macOS Hide/etc.
- Quit Cmd+Q

Stream menu should be state-aware: Search/Refresh when idle, Cancel while connecting/reconnecting, Stop Stream while Playing, future Mute/Synchronize.

Help menu direction:

- Open Logs Folder (`L`)
- Relaunch in Debug Mode
- Relaunch in Verbose Mode
- future Help / GitHub / Report Issue

Manual GitHub Releases update check is sufficient initially; no background updater required for first pass.

---

## Planned first-release — in-app diagnostic log viewer

Normal mode: **absent** from layout entirely.

Debug: live Debug log visible.

Verbose: live Verbose log visible.

Requirements when visible:

- never miss events
- read-only/selectable native text
- monospace scroll view
- Copy / Cmd+C / Select All
- follow tail only when already at bottom
- scrolling upward disables forced autoscroll

Collapse/expand behavior is intentionally undecided.

---

## Host console / TUI direction

**Status: Future UX direction**

Host remains a lightweight console application; no dedicated GUI is currently desired.

Prefer a restrained custom terminal UI over immediately adopting a heavyweight ncurses-style/full-screen framework:

```text
logs scroll above
-----------------
S Settings   L Logs   H Help   Ctrl+C Quit
```

Footer/legend may change with state. `H` could expand upward and reduce log runway. Adopt a real TUI library only if custom redraw/resize/focus complexity justifies it.

---

## Logging implementation / performance

**Status: Future architecture-audit/refactor consideration**

Current logging is synchronous. Consider later:

```text
producer threads -> bounded event queue -> one logging worker -> terminal/file/UI sinks
```

Goals: never block media/supervision on slow disk/terminal, no thread-per-message, no unbounded queue, preserve critical structured events, allow pathological raw Verbose chatter to be throttled/dropped if necessary.

Do not build this without evidence or as pre-emptive optimization.

Hardening backlog: active `pgc-client-latest.log` manually deleted during a running session.

---

## Pre-release architecture / shared-core audit

**Status: Planned before first public release**

Use the strongest available coding/reasoning model. First pass analysis-only.

Audit:

- duplication and shared invariants encoded separately
- overly broad modules
- platform leakage
- dead/stale code
- ownership/coupling
- unnecessary allocations/copies/polling
- meaningful performance issues
- shared-crate opportunities
- harmful/premature abstractions
- rationale/archaeology gaps

Classify findings:

- Low-risk / high-value
- Medium-risk
- Architectural
- Defer

Likely candidates to review: logging, protocol/versioning, discovery constants/TXT parsing, configuration models, platform/build metadata. Do not abstract merely because two files look similar.

---

## Versioning implementation

**Status: Planned**

Policy is in `PGC-VERSIONING.md`.

Implement:

- independent Client SemVer
- independent Host SemVer
- independent protocol version
- platform build/revision numbers
- macOS metadata mappings
- Windows product/file metadata
- About/debug/startup output
- explicit `protocol_version` discovery metadata when appropriate

Build channels: Development / Nightly / Beta / Release. They are provenance, not compatibility. Do not infer channel from branch until the versioning pass deliberately defines that behavior.

---

## Release structure / packaging

**Status: Planned**

Keep Host and Client as separate downloadable applications by role/platform, for example:

```text
PGC-Host-Windows-x64
PGC-Client-Windows-x64
PGC-Client-macOS-arm64
```

Names are illustrative until manual releases establish conventions.

Manual releases first to learn artifact names, tags, release-note structure, checksums/signing expectations, and independent Host/Client version behavior. Automate with GitHub Actions later.

Windows packaging eventually needs Host, MediaMTX, compatible FFmpeg, discovery/service setup, firewall/config, dependency preflight, capture-source selection, and background startup.

macOS should eventually remove Homebrew as a hard runtime dependency where licensing/distribution permits.

---

## First public release milestone

The first meaningful `master -> main` PR should represent an intentionally structured public release milestone, after:

- Host portability/capture-source baseline
- pre-release architecture/shared-core cleanup
- versioning
- first-release Client UX
- packaging
- hands-on stability validation
- documentation/release review

Before that release, update repository description/README opening/Quick Start/download-install/supported-platform framing.

Working explanation:

> Portable Game Caster turns a capture device connected to one computer into a lightweight, discoverable LAN video source another device can watch locally.

Gaming is the flagship use case, but the underlying idea is local casting of arbitrary captured video.

Product wedge:

- LAN-first
- capture-first
- viewing rather than remote control
- commodity hardware
- demand-driven low idle footprint
- compressed / Wi-Fi-friendly
- automatic discovery
- lightweight dedicated Host and Client
- cross-platform direction
- no cloud/account requirement
- no OBS workflow requirement

---

## Licensing / commercial boundary

**Status: Undecided — resolve before first meaningful public release**

- current GPL-3.0-only choice was provisional
- current preference is free/source-visible community/home use while reserving commercial application/use
- PolyForm Noncommercial is a conceptual candidate; that would be source-available, not OSI open source
- no final license decision yet; `LICENSE` unchanged
- contributor/relicensing strategy must be decided before outside contributions under a commercial dual-track model
- legal/IP structure requires professional legal review

The community repo is not the commercial/event-product roadmap. Private commercial/event planning belongs separately and should not leak into public roadmap/business details.

---

## Validation infrastructure

**Status: Planned — high priority after first release**

Desired model:

- GitHub Actions / CI for generic build/unit checks
- MCP-style or self-hosted real-machine validation for Windows/Linux/macOS
- hardware-aware checks for capture devices, NVENC, process lifecycle, etc.
- remote driver can request validation without using a Git commit merely as transport

CI complements real-hardware validation; it does not replace it.

---

## Resource profiling

**Status: Planned / low priority**

Later measure idle CPU wakeups, GPU activity, power, USB/PCIe/capture activity, network traffic, long-duration memory behavior, and Debug/Verbose logging cost.

Prototype informal baseline: idle ~35 MB, active ~240 MB / ~11% GPU.

---

## Stream / Client future controls

Planned or future:

- local Client Mute (local playback only; do not affect Host/other viewers/Discord capture semantics inadvertently)
- Synchronize / return to freshest safe live edge
- stream information (resolution/rate/bitrate/health where available)
- dedicated PGC player wrapper/title
- native macOS power assertion while playing
- stronger network/connection health model

---

## Dependency / preflight

Host should detect/explain missing MediaMTX, FFmpeg, bad executable path, unsupported FFmpeg/NVENC, bad config, missing capture/audio devices.

Client should detect/explain missing ffplay/Homebrew/ffmpeg-full today and future packaged dependency failures.

Diagnostics should say paths searched and why startup failed.

---

## Recording / hardware expansion

Future:

- archival recording independent from live profile
- simultaneous live + archival capture
- AVerMedia GC575 integration
- PC Line In / surround-audio strategy
- capture-card health/wake handling

---

## Platform expansion

Future:

- Windows Client
- Linux Client
- Linux Host
- Steam Deck
- macOS Host where useful
- Android
- iOS/iPadOS

Shared SemVer should only be claimed where feature/behavior parity is real.

---

## Branch / integration workflow

**Status: Active**

Working branches should use frequent focused ticket commits after engineering validation. Those commits preserve implementation history; they do not claim acceptance.

Hands-on/native validation defines platform/feature acceptance. Larger platform checkpoint commits can carry richer archaeological summaries. Accepted feature branches receive a documentation reconciliation before merge/PR into `master`.

Branch scope should remain coherent. If adjacent work becomes an independently reviewable feature/fix, changes another product area, deserves its own acceptance/release-note story, or begins to dominate the branch, spin it into a sibling/follow-on branch rather than letting the current branch become a generic improvement bucket.

Current logging branch taught this lesson: some Mac interaction improvements were useful and accepted, but a future similarly coherent UX expansion should likely move to a branch such as `feature/client-ux-improvements` rather than continuing indefinitely under a logging branch.

At larger stable milestones:

```text
master -> project-level review -> PR into main
```

---

## Completed / accepted foundations

- native macOS AppKit Client shell
- macOS mDNS discovery and same-Host recovery
- truthful state model / WaitingForStream semantics
- contextual Cancel / Stop / Retry controls
- deadline-driven countdowns
- ffplay metadata parsing and initial stalled-media watchdog
- Windows native mDNS advertiser / MediaMTX supervisor
- Windows singleton guard
- native Windows Host FFmpeg lifecycle ownership
- demand-signal helper architecture
- Job Object containment
- demand-driven no-encoder idle state
- source-verified MediaMTX demand-model diagnosis
- macOS Client unified logging/session files/readable STATE breadcrumbs
- macOS utility shelf, Logs shortcut, interaction hover/pressed feedback
- macOS keyboard/focus model and development UI self-test

Not yet accepted/completed: Windows Host side of the unified logging/console transition.
