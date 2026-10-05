# Portable Game Caster Decision Log

This file preserves durable product, architecture, and development-process decisions and why they were made. It is not a changelog.

Implementation facts and landmines belong in `PGC-DEV-NOTES.md`. Future work belongs in `PGC-ROADMAP.md`. Current accepted architecture belongs in `PGC-ARCHITECTURE.md`.

---

## 2026-09 — Portable Game Caster is a local broadcast/listener system

### Decision

Clients consume a local Host stream without tightly owning Host sessions.

### Rationale

The stream may have multiple readers and should behave more like tuning into a local broadcast than opening an exclusive remote session.

### Implications

- Client Stop Stream and Cancel are local.
- Host lifecycle does not depend on one Client UI state.
- No Client -> Host cancel/session message is required.
- Multiple Clients should eventually be possible.

---

## 2026-09 — SRT publisher -> MediaMTX -> RTSP/TCP Client is the working transport topology

### Decision

```text
FFmpeg -> SRT localhost -> MediaMTX -> RTSP/TCP LAN -> Client
```

### Rationale

Direct SRT playback was less desirable operationally, while RTSP ingest into MediaMTX produced capture/backpressure and packet-loss problems. The SRT-publish / RTSP-read split is the best tested topology.

### Implication

Do not casually reintroduce RTSP ingest without a fundamentally different solution.

---

## 2026-09 — Playing means confirmed media, not connectivity

### Decision

Client may enter `Playing` only after actual decoded media flow is confirmed.

### Rationale

A reachable RTSP server, open socket, or running player can all exist while no usable media is flowing. User-facing state must describe reality.

---

## 2026-09 — Fresh connection and recovery states are distinct

Fresh/cold attempts never use reconnect states. Reconnect states are reserved for recovery after a previously healthy Playing session.

"Reconnecting" should only appear when something actually worked before.

---

## 2026-09 — Cancel and Stop Stream are explicit local controls

Contextual Client controls are Search, Cancel, Stop Stream, Retry, and Quit.

Cancel immediately returns to Idle, invalidates stale worker messages, and kills any race-started player. Stop Stream terminates playback and returns to Idle without triggering recovery.

---

## 2026-09 — One Host process per Windows machine

Only one Host may run per machine.

```text
Global\PortableGameCasterHost.v1
```

The mutex name identifies the singleton contract, not Host SemVer. Multiple Hosts on one LAN remain valid.

---

## 2026-10 — Client media health must detect frozen media

A Client cannot remain Playing indefinitely simply because ffplay and RTSP remain alive.

Current interim health ignores the `-sync ext` master clock and watches the remaining ffplay status. Roughly five seconds without non-master progress triggers recovery.

Known future gaps: no-telemetry detection and video/audio liveness separation.

---

## 2026-10 — Visible countdowns are deadline-driven

Reconnect, warm-up, and discovery countdowns derive from monotonic deadlines rather than worker-loop cadence so network/mDNS work cannot distort visible seconds.

---

## 2026-10 — UTC/Zulu is the canonical diagnostic timestamp

PGC diagnostics use UTC/Zulu with millisecond precision so Host and Client logs from different machines can be correlated directly.

---

## 2026-10 — Host capture remains demand-driven

### Decision

The Host must not keep FFmpeg, capture hardware, or NVENC active merely because the Host process is running.

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

PGC should be effectively unnoticeable when nobody is watching. A few seconds of first-stream startup is an acceptable trade.

Initial Windows validation supported this requirement: roughly 35 MB idle memory with effectively no CPU/GPU use on the prototype machine.

---

## 2026-10 — MediaMTX does not own FFmpeg lifecycle

### Decision

Native Rust Host owns FFmpeg; MediaMTX remains relay and demand infrastructure.

### Rationale

MediaMTX 1.21.1 source inspection showed that waiting DESCRIBE requests are not active readers and close-after can expire while a recovering Client is still waiting. Those semantics do not provide the lifecycle ownership guarantees PGC needs.

### Accepted model

```text
RTSP reader demand
-> MediaMTX runOnDemand
-> pgc-host-windows.exe --demand-signal
-> held localhost connection to running Host
-> Host starts/owns FFmpeg
-> FFmpeg publishes SRT to MediaMTX
-> MediaMTX serves RTSP readers
```

Do not return FFmpeg ownership to MediaMTX scripts merely because `runOnDemand` remains in the configuration.

---

## 2026-10 — PID files and PowerShell are not the active ownership model

Native Host owns child/process handles directly. Old PowerShell start/stop scripts and PID-file ownership are legacy/deprecated and may remain only for archaeological/reference value.

---

## 2026-10 — Host-owned child processes use Windows Job Object containment

MediaMTX and FFmpeg should be assigned to a kill-on-close Job Object on a best-effort basis so abnormal Host termination does not leave owned infrastructure behind.

---

## 2026-10 — Unified logging semantics across Host and Client

### Decision

Host and Client share Quiet / Normal / Debug / Verbose semantics and a common presentation model.

- Quiet: terminal silent; support/session log still written.
- Normal: important lifecycle/user-facing production diagnostics.
- Debug: structured event-driven internal diagnostics, no raw subprocess firehose.
- Verbose: Debug plus raw external provenance and deeper internal decision details.

### Presentation

- UTC/Zulu millisecond timestamps
- stable fixed-width source/category fields
- readable STATE lines at Normal/Debug
- raw internal state only at Verbose
- no ANSI in files/redirects
- Verbose always identifies source
- explicit user actions and final state transitions leave breadcrumbs

### Status

macOS Client side is UX Approved. Windows Host side is implemented and awaiting native Windows validation on `feature/host-client-logging`.

---

## 2026-10 — Observability and smoke tests are part of feature delivery

Every feature should decide what its new user actions, state transitions, failure paths, and background subsystems log at Normal, Debug, and Verbose.

Development-only smoke-test tools are encouraged when they reduce otherwise unverifiable behavior, stay contained, do not alter production behavior, and are compiled/kept out of production where appropriate. They never replace hands-on UX acceptance.

---

## 2026-10 — Repository documentation is canonical shared project memory

Repository docs are the shared reference point for the user/product owner, ChatGPT, coding agents, and future contributors.

Implementation reports flag documentation impact. Documentation may be consolidated over several small tickets, but accepted working branches must be reconciled before merge into `master`, and `master` gets a broader review before promotion to `main`.

---

## 2026-10 — Commit messages and code comments are archaeological documentation

Commit history and non-obvious comments should let a future maintainer reconstruct what changed, why, what it replaced, and what constraints mattered without access to the original conversation.

Small ticket commits can be concise. Larger platform acceptance, architectural transition, and integration commits should carry richer rationale.

---

## 2026-10 — Working branches use frequent focused commits; acceptance is a separate checkpoint

### Decision

Working branches should preserve progress with frequent, focused commits after engineering validation. One completed ticket will often map to one commit.

A ticket commit records that an implementation checkpoint built/tested as reported. It is not proof that UX, hardware behavior, or the whole feature has been accepted.

Hands-on/native validation remains required before the relevant platform or feature is considered accepted and before merge into `master`.

Agents may commit only when the current ticket/workflow explicitly permits it. Feature-branch context alone is not permission.

### Rationale

Short-lived branches exist partly to make incremental history cheap and useful. Avoiding commits until the end produces oversized changes that are harder to review, bisect, and understand. If hands-on testing finds a defect, a later fix commit is healthy history rather than evidence that the earlier checkpoint should never have existed.

### History levels

1. **Ticket commit** — small, focused, concise.
2. **Platform/acceptance checkpoint** — richer archaeological summary after native/hands-on validation.
3. **Feature integration / PR** — high-level feature narrative with docs reconciled.

---

## 2026-10 — Development uses short-lived working branches, master integration, and main milestones

```text
main    stable milestone/release baseline
master  integrated accepted development
feature/fix/refactor/spike branches  active work
```

Work branches can be iterative/destructive and contain many useful commits. `master` represents accepted integrated work; `main` moves at larger stable/release milestones.

The first meaningful `master -> main` PR should represent the first intentionally structured public release milestone, not merely completion of versioning.

---

## 2026-10 — Working branch scope should remain coherent

### Decision

A working branch has one primary intent. Adjacent work stays only when it is directly required to implement, validate, diagnose, or make that intent usable.

When new work becomes independently reviewable, changes another product area, deserves its own release-note/acceptance story, or begins to dominate the branch diff, create a sibling/follow-on branch instead of silently expanding scope.

### Rationale

`feature/host-client-logging` grew to include substantial macOS interaction polish because the logging controls exposed nearby UX issues. The result is accepted and useful, but it demonstrated how easily a feature branch can become a generic improvement bucket.

### Implication

Do not rewrite useful history solely to make old branch boundaries look pure. Apply this guardrail prospectively. A future coherent Client interaction pass would deserve a branch such as `feature/client-ux-improvements`.

---

## 2026-10 — Cross-platform features have one driver; other machines validate

One driver owns feature architecture. Other machines/agents validate natively and fix platform-specific defects without independently redesigning the feature.

Manual patch/snapshot handoff is acceptable while remote validation infrastructure does not yet exist. Handoffs include untracked files, baseline, file lists, and validation caveats. Git history, not temporary copies, is authoritative.

Long-term: CI for generic validation plus MCP/self-hosted real-machine validation for platform/hardware paths.

---

## 2026-10 — Host and Client remain separate applications

Host and Client ship as separate role/platform artifacts. Shared underlying crates are desirable where justified, but no unified Host+Client shell should be introduced merely to combine them.

---

## 2026-10 — First public release includes a deliberate architecture/readiness pass

Before the first meaningful public release, complete a deep analysis-first architecture audit, Host portability baseline, versioning, first-release Client UX, packaging, and hands-on stability validation.

The audit should identify duplication, platform leakage, ownership/coupling, stale code, meaningful performance issues, good shared-crate candidates, and harmful/premature abstractions before refactoring.

---

## 2026-10 — Community and commercial planning remain separate

The community repository is not the commercial/event-product roadmap. Commercial/event planning lives separately and private business/product details do not belong here.

The current GPL license choice is provisional. A source-visible/noncommercial community model such as PolyForm Noncommercial has been discussed as a candidate, but no final license decision has been made. Any future dual-track/commercial licensing and contributor/relicensing strategy requires explicit decisions and professional legal review.
