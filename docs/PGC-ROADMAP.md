# Portable Game Caster Roadmap / Backlog

This backlog is ordered approximately by current priority, not by strict release commitment.

## Status vocabulary

- `Planned`
- `In Progress`
- `Awaiting Hands-on Validation`
- `Awaiting Hands-on UX Validation`
- `UX Approved`
- `Done`

---

## Immediate — Windows Host native FFmpeg ownership

**Status: In Progress**

Replace MediaMTX/PowerShell FFmpeg ownership with native Rust Host ownership while preserving demand-driven idle behavior.

Requirements:

- Host remains lightweight while idle
- FFmpeg stopped while no viewers exist
- capture device unopened while idle
- NVENC idle while idle
- no gameplay publisher bandwidth while idle
- MediaMTX may provide demand information
- MediaMTX must not own FFmpeg lifecycle
- Host directly launches FFmpeg
- Host retains FFmpeg process handle
- Host monitors expected/unexpected exit
- Host restarts FFmpeg while demand remains
- Host stops FFmpeg when demand ends
- Host guarantees no more than one owned FFmpeg
- Host shutdown leaves no orphan FFmpeg or MediaMTX
- FFmpeg-kill recovery works
- MediaMTX-kill recovery works
- rapid/awkward failure timing does not produce duplicate encoders
- remove PowerShell/PID-file dependence from active lifecycle after validation

Current hardcoded Elgato/NVENC profile is acceptable for this milestone.

Capture portability is a later milestone.

---

## Immediate — unified Host + Client logging UX / log levels

**Status: Planned — expected immediately after Host lifecycle work**

Create one shared logging semantics and presentation model across Host and Client.

### Modes

#### Normal

- important lifecycle/user-facing events only
- lowest practical overhead
- warnings/errors
- no raw external-process firehose
- intended production/default mode

#### Debug

- structured PGC state/lifecycle diagnostics
- discovery/connection details
- process ownership/recovery decisions
- health state
- still event-driven
- no raw FFmpeg/ffplay/MediaMTX firehose

#### Verbose

- Debug plus raw external output
- external source identity mandatory
- greater console I/O/formatting overhead is acceptable

### Presentation

- UTC/Zulu timestamps with millisecond precision
- timestamp dim/grey in interactive terminals
- fixed-width columns
- message text starts at one consistent position
- stable category colours
- category colours shared between Host and Client where names overlap
- Debug may omit redundant `[PGC]`
- Verbose explicitly labels every source:
  - `[PGC]`
  - `[FFPLAY]`
  - `[FFMPEG]`
  - `[MTX]`
  - future sources as needed
- external output is never ambiguous
- redirected/file output remains plain text
- terminal styling centralized
- preserve timestamped MediaMTX observability
- restore useful Host terminal colour
- preserve FFmpeg progress readability where possible

Potential PGC categories:

- APP
- STATE
- CONNECT
- DISCOVERY
- PLAYER
- STREAM
- HEALTH
- RECONNECT
- HOST
- CAPTURE
- ENCODER
- DEMAND

---

## Client recovery / truthfulness

**Status: Core UX approved; broader soak testing remains**

Accepted:

- Playing only after decoded media confirmed
- reconnect states only after a healthy Playing state is lost
- restored RTSP moves to WaitingForStream
- cold timeout:
  - `Stream did not start.`
- recovery timeout:
  - `Stream did not resume.`
- monotonic deadline-driven reconnect countdown
- monotonic deadline-driven warm-up countdown
- improved discovery countdown cadence
- Cancel implemented and UX-approved
- Stop Stream implemented and UX-approved
- friendly Host identity header implemented
- stalled-media watchdog exits Playing after approximately five seconds of frozen ffplay status progress

Remaining:

- long healthy-session soak for false-positive stall detection
- detect ffplay ceasing telemetry entirely
- distinguish video liveness from audio liveness
- evolve text-parsed media health into a stronger first-class health model

---

## Client session-control / observability polish

**Status: Planned**

Add concise Debug breadcrumbs for explicit user actions:

- Search
- Retry
- Cancel
- Stop Stream
- Quit
- future Synchronize
- future configuration changes

Preserve:

- stale-worker invalidation
- local Stop/Cancel behavior
- no Client -> Host session-control coupling

---

## Windows Host portability milestone

**Status: Planned after native lifecycle ownership**

Remove prototype capture hardcoding.

Host should eventually:

- enumerate video capture sources
- enumerate audio capture sources independently
- allow selection
- persist configuration
- validate unavailable/missing devices
- surface capture health
- support arbitrary DirectShow devices
- support future non-Windows capture backends
- provide lightweight native Host configuration UI

Potential Host controls:

- Video source
- Audio source
- Encoder
- Resolution/profile
- Status
- Start/stop or service controls where appropriate

---

## Encoder/backend support

**Status: Planned**

Profiles/backends:

- modern NVIDIA
- Pascal/legacy NVIDIA
- Intel QSV
- AMD AMF
- x264 fallback

Investigate:

- compatible FFmpeg distribution strategy
- legacy vs modern NVIDIA requirements
- runtime encoder capability detection
- fallback ordering
- licensing/distribution considerations

Prefer profile selection over separate Host builds.

---

## Real media-flow health

**Status: Incremental / ongoing**

Differentiate:

- process alive
- relay reachable
- transport alive
- publisher alive
- video advancing
- audio advancing
- network healthy

Future Host health should distinguish:

- capture failure
- encoder failure
- publisher failure
- MediaMTX failure
- network/client failure

Future Client health should support:

- video frame progress
- audio progress
- last-media timestamp
- stalled video
- missing audio
- degraded transport/network

---

## Client stream UX

**Status: Planned**

Connected-state stream information:

- duration
- bitrate
- resolution
- frame rate
- useful connection/network health

Future controls:

- `Synchronize`
  - return playback to freshest safe live edge
  - avoid audio warping/stuttering
- dedicated PGC player wrapper/window
- useful player title:
  - `Portable Game Caster Stream — <hostname>`
- native macOS power assertion during active playback

---

## Client discovery robustness

**Status: Planned**

- investigate rare immediate re-search misses
- test rapid Stop Stream -> Search cycles
- potentially retry/reuse mDNS browse before surfacing failure
- preserve distinction:
  - no service found
  - service found but address unresolved
- multiple-Host selection
- preferred Host memory/selection behavior

---

## Dependency validation

**Status: Planned**

### Host

Detect and explain:

- MediaMTX missing
- FFmpeg missing
- invalid executable path
- launch failure
- unsupported FFmpeg/NVENC combination
- bad MediaMTX config
- missing capture device
- missing audio device

### Client

Detect and explain:

- ffplay missing
- Homebrew missing
- ffmpeg-full missing
- invalid `PGC_FFPLAY_PATH`
- player launch failure

Diagnostics should explain paths searched and why startup failed.

---

## Versioning implementation

**Status: Planned**

Policy exists in `PGC-VERSIONING.md`.

Implementation:

- independent Client SemVer
- independent Host SemVer
- independent protocol version
- platform build/revision numbers
- macOS version metadata
- Windows product/file metadata
- About/debug output:
  - version
  - build
  - platform
  - protocol
- evolve mDNS TXT:
  - from `version=1`
  - toward explicit `protocol_version=1`

---

## Packaging / installation

**Status: Planned**

### Windows

Installer/package should eventually handle:

- PGC Host
- MediaMTX
- compatible FFmpeg
- discovery/service setup
- firewall/configuration
- dependency validation
- capture-device selection
- background startup

### macOS

Eventually remove Homebrew as a hard runtime dependency where licensing/distribution permits by bundling or managing playback dependencies.

---

## Background/service lifecycle

**Status: Planned**

- proper Windows background service/helper
- startup at login/system startup
- clean child-process ownership
- low idle footprint
- dependency preflight
- clean shutdown
- user-visible status/control surface where appropriate

---

## Latency

**Status: Planned optimization**

- reduce end-to-end latency further
- current MediaMTX relay adds roughly ~0.5 seconds compared with prior direct point-to-point testing
- preserve stable current transport until a clearly better option is demonstrated
- do not trade reliability for marginal latency improvements

---

## Known stream cleanup items

**Status: Deferred**

- SRT ACKACK noise
- cold-start H.264 PPS warnings
- cold-source startup polish

Do not reintroduce RTSP ingest unless a fundamentally new approach addresses the prior DirectShow backpressure/choppiness problem.

---

## Recording / archival

**Status: Planned**

- archival recording
- simultaneous live + archival capture
- AVerMedia GC575 integration
- surround audio capture strategy
- archival quality profiles independent from live stream profile

---

## Hardware/audio expansion

**Status: Future**

- AVerMedia Live Gamer 4K 2.1 / GC575
- PC Line In audio path where needed
- surround audio
- HDMI/AVR compatibility matrix
- additional AVR configurations
- capture-card health/wake handling

---

## Platform expansion

**Status: Future**

- Windows Client
- Linux Client
- Linux Host
- Steam Deck
- macOS Host where useful
- Android
- iOS / iPadOS

Shared product SemVer should only be claimed where feature parity is real.

---

## Documentation system

**Status: Active**

Repository docs are canonical shared project memory.

Workflow:

- decisions recorded in DECISIONS
- roadmap state maintained continuously
- implementation reports flag documentation impact
- architecture/dev notes updated after approved milestones
- code and documentation preferably committed together

---

## Completed / accepted foundations

- native macOS AppKit Client shell
- macOS mDNS discovery
- same-host reconnect behavior
- Windows native mDNS advertiser
- Windows MediaMTX supervisor
- Windows machine-wide single-instance guard
- truthful Client state model
- WaitingForStream semantics
- contextual Cancel / Stop Stream / Retry controls
- friendly persistent Host identity
- deadline-driven countdowns
- Client UTC millisecond timestamps
- ffplay stream metadata parsing
- initial stalled-media watchdog
- MediaMTX lifecycle instrumentation
- source-verified diagnosis of runOnDemand demand-model mismatch
- best-effort wider Windows Host console