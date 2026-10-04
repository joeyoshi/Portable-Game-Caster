# Portable Game Caster Decision Log

This file preserves durable product and architecture decisions and, most importantly, why they were made.

It is not a changelog.

Implementation details belong in `PGC-DEV-NOTES.md`. Future work belongs in `PGC-ROADMAP.md`. Current accepted architecture belongs in `PGC-ARCHITECTURE.md`.

---

## 2026-09 — PGC is a local broadcast/listener system

### Decision

Portable Game Caster is fundamentally a local-network broadcast/listener architecture.

Clients consume a Host stream without tightly owning Host sessions.

### Rationale

The stream may have multiple readers and should behave more like tuning into a local broadcast than opening an exclusive remote session.

### Implications

- Client Stop Stream is local.
- Client Cancel is local.
- No Client -> Host cancel message is required.
- Host lifecycle should not depend on one Client's UI state.
- Multiple Clients should eventually be possible.

---

## 2026-09 — SRT publisher -> MediaMTX -> RTSP Client remains the working transport topology

### Decision

Use:

```text
FFmpeg
-> SRT localhost
-> MediaMTX
-> RTSP/TCP LAN
-> Client
```

### Rationale

Direct SRT playback worked but created protocol/logging behavior that was less desirable.

RTSP ingest into MediaMTX was experimentally worse for capture stability.

The current SRT-publish / RTSP-read split is the best tested combination.

### Rejected alternatives

RTSP/TCP ingest:

- severe DirectShow backpressure
- choppy playback

RTSP/UDP ingest:

- packet loss
- invalid FU-A behavior
- choppy playback

### Implication

Do not casually reintroduce RTSP ingest without a fundamentally new approach.

---

## 2026-09 — Playing means confirmed media, not connectivity

### Decision

The Client may enter `Playing` only after actual decoded media flow has been confirmed.

### Rationale

A reachable RTSP server, an open socket, or a running ffplay process can all exist while no usable media is flowing.

User-facing state must describe reality.

### Implication

The Client uses a distinct `WaitingForStream` state after connectivity is restored but before media is confirmed.

---

## 2026-09 — Fresh connection and recovery states are distinct

### Decision

Fresh/cold connection attempts never use reconnect states.

`ReconnectingStream` and `ReconnectingHost` are reserved for recovery after a previously healthy Playing session.

### Rationale

"Reconnecting" implies that something previously worked.

Using reconnect language on first connection is misleading.

---

## 2026-09 — Cancel and Stop Stream are explicit local session controls

### Decision

Client controls are contextual:

- Search
- Cancel
- Stop Stream
- Retry
- Quit

### Rationale

Users need immediate control over long connection/recovery operations without having to quit the app.

### Implications

Cancel:

- returns to Idle
- invalidates stale worker messages
- kills race-started ffplay

Stop Stream:

- kills ffplay
- returns to Idle
- does not enter reconnect

---

## 2026-09 — One Host process per machine

### Decision

Only one PGC Host may run on a Windows machine.

### Implementation identity

```text
Global\PortableGameCasterHost.v1
```

### Rationale

Multiple Host supervisors on one machine would compete for MediaMTX ports, capture hardware, discovery identity, and child-process ownership.

### Important distinction

Multiple PGC Hosts on the same LAN are valid.

The mutex name is a singleton-contract identifier, not Host SemVer.

---

## 2026-10 — Client media health must detect frozen media

### Decision

A Client cannot remain Playing indefinitely simply because ffplay and RTSP remain alive.

### Rationale

Testing showed that ffplay can stay open and continue printing status while the actual source is frozen.

### Interim implementation

After MediaStarted:

- ignore the `-sync ext` master clock
- monitor the remaining ffplay status fields
- approximately five seconds of no change triggers recovery

### Known limitations

Future health work should detect:

- no ffplay telemetry at all
- video-only freeze while audio continues

---

## 2026-10 — Visible countdowns are deadline-driven

### Decision

Reconnect, warm-up, and discovery countdowns derive from monotonic deadlines.

### Rationale

Loop-based countdown values were distorted by network probes, mDNS calls, and worker scheduling.

### Implication

Do not derive visible countdown cadence from worker-loop iteration timing.

---

## 2026-10 — UTC/Zulu is the canonical diagnostic timestamp

### Decision

PGC diagnostics use UTC/Zulu timestamps with millisecond precision.

Example:

```text
[07:23:18.492Z]
```

### Rationale

Host and Client may be on separate machines and future users may share logs remotely.

Universal timestamps make cross-machine correlation direct and unambiguous.

### Implication

Do not convert logs to local wall-clock time merely for familiarity.

---

## 2026-10 — Host capture remains demand-driven

### Decision

The Host must not keep FFmpeg/capture/NVENC active simply because the Host process is running.

### Idle target

```text
MediaMTX: running
mDNS: running
FFmpeg: stopped
capture device: unopened
NVENC: idle
publisher traffic: none
```

### Rationale

PGC should be effectively unnoticeable when nobody is watching.

An always-on encoder would unnecessarily consume:

- capture-card/USB or PCIe bandwidth
- CPU memory bandwidth
- GPU/NVENC resources
- power
- local network/media pipeline activity

A few seconds of startup is acceptable.

### Implication

An always-on publisher is not an acceptable simplification of Host lifecycle.

---

## 2026-10 — MediaMTX should not own FFmpeg lifecycle

### Decision

Move FFmpeg ownership into the native Rust Host.

MediaMTX remains relay/demand infrastructure.

### Rationale

Source inspection of MediaMTX 1.21.1 confirmed that its runOnDemand demand model does not match PGC's recovery semantics.

In particular:

- a waiting DESCRIBE is not counted as a reader
- close-after may expire while the Client is still waiting
- runOnDemand restart uses a fixed delay
- restart and close timers can collide

These are not merely PowerShell bugs.

### Target

```text
PGC Host
|- MediaMTX
`- FFmpeg
```

instead of:

```text
PGC Host
`- MediaMTX
   `- PowerShell
      `- FFmpeg
```

### Implication

Do not spend significant engineering effort perfecting the PowerShell/PID-file bridge once native ownership is available.

---

## 2026-10 — PID files are not the long-term ownership model

### Decision

Native Host FFmpeg ownership must use direct child/process handles.

### Rationale

PID files are useful diagnostics but fragile as the authority for process ownership.

Potential problems include:

- stale PID
- PID reuse
- start/stop ordering races
- ownership ambiguity

### Implication

PID files may remain temporarily for diagnostics/rollback but should not define primary ownership.

---

## 2026-10 — Unified logging semantics across Host and Client

### Decision

Host and Client should share the same conceptual logging modes and console presentation.

### Modes

Normal:

- important lifecycle only
- low overhead
- no raw external output

Debug:

- structured PGC diagnostics
- event-driven
- practical for troubleshooting

Verbose:

- Debug plus raw external sources
- explicit source identity on every line

### Presentation

Debug:

```text
[timestamp] [CATEGORY]    message
```

Verbose:

```text
[timestamp] [SOURCE] [CATEGORY]    message
```

External source example:

```text
[timestamp] [FFPLAY]               message
[timestamp] [FFMPEG]               message
[timestamp] [MTX]                  message
```

### Rationale

Logs should be visually scannable and never ambiguous about who produced a line.

### Styling rules

- timestamps dim/grey
- category/source colours stable
- fixed-width fields
- message text aligned
- no ANSI in redirected output
- Debug may omit redundant `[PGC]`
- Verbose always includes explicit provenance

---

## 2026-10 — Repository documentation is canonical shared project memory

### Decision

Repository documentation is the shared reference point for:

- user/product owner
- ChatGPT
- coding agents
- future contributors

### Workflow

1. Decisions may be recorded immediately.
2. Experimental code does not automatically become architecture.
3. Every coding report assesses documentation impact.
4. Approved implementation updates Architecture and Dev Notes.
5. Roadmap reflects priority/status continuously.
6. Prefer committing accepted code and documentation together.

### Rationale

The project has grown beyond what should live only in conversational memory.

### Implication

A milestone is not considered fully synchronized until its documentation impact has been reviewed.