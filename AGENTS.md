# Portable Game Caster — Agent Guide

Portable Game Caster (PGC) is a local-network console gameplay streaming/capture system under the JoeYoshi banner.

Use "Portable Game Caster" in prose and user-facing text. "PGC" is fine in code identifiers, log provenance (`[PGC]`), file names, and document shorthand.

## Project identity

- License: `GPL-3.0-only` today, but provisional. The community license must be explicitly decided before the first meaningful public release. Do not change `LICENSE` without that decision.
- macOS bundle ID: `com.joeyoshi.portablegamecaster`
- Primary language: Rust
- Repository: `joeyoshi/PortableGameCaster`
- Integrated development branch: `master`
- Stable milestone/release branch: `main`

## Branch model

```text
main
  stable milestone / release baseline

master
  integrated, accepted development state

feature/*, fix/*, refactor/*, spike/*
  short-lived working branches
```

Working branches should preserve useful incremental history. A completed engineering ticket will often map cleanly to one focused commit after its engineering validation. Small ticket commits may use concise one-line subjects. Richer archaeological messages belong at platform acceptance checkpoints, major architectural transitions, and feature integration.

A commit records an implementation checkpoint. It does **not** by itself mean UX, hardware behavior, or the whole feature has been accepted.

Agents may commit only when the current ticket/workflow explicitly permits it. Merely being on a feature branch is not permission.

Prefer non-squash integration when intermediate commits carry useful history.

## Validation and acceptance

Keep these concepts separate:

- **Implemented**: code exists.
- **Engineering Validated**: appropriate builds/tests/synthetic checks passed.
- **Awaiting Hands-on Validation / UX Validation**: engineering checks passed, human/native behavior still needs review.
- **UX Approved**: user-visible behavior has been hands-on accepted.
- **Done**: feature/platform scope is accepted and synchronized.

User-visible, hardware-dependent, and platform-specific work still requires hands-on/native validation before that platform or feature is considered accepted or merged into `master`.

If hands-on testing finds a defect after a ticket commit, fix it in a later commit. Do not avoid useful intermediate history merely to make every commit look final.

## Branch-scope guardrails

A working branch should have one primary intent. Small adjacent changes may stay on that branch when they are directly required to implement, validate, diagnose, or make the primary feature usable.

Before broadening scope, ask whether the new work:

- is independently reviewable as a feature/fix,
- changes an unrelated subsystem or product area,
- deserves its own release-note bullet or acceptance pass,
- is starting to dominate the branch diff or ticket sequence, or
- can be cleanly based on the current branch without blocking its completion.

If several are true, propose a sibling/follow-on branch instead of silently expanding the current one. For example, interaction polish discovered during logging work may belong on `feature/client-ux-improvements` once it becomes a coherent feature of its own.

Do not retroactively rewrite useful history merely to make branch boundaries look perfect. Record the scope lesson and apply it to the next branch.

## Cross-platform workflow

One implementation owner/driver owns a feature's architecture. Other machines/agents act as platform validators: they build/test natively, diagnose platform-specific defects, and avoid independent redesign. The role follows the feature, not the OS.

When uncommitted target-platform code must move between machines:

1. implement on the driver machine;
2. engineering-test locally where possible;
3. hands-on validate the driver's platform where relevant;
4. commit only the validated source-platform work;
5. package the remaining target-platform delta as patch and/or file snapshot;
6. include untracked files, baseline commit, file list, and validation caveats;
7. transfer to the target machine;
8. perform native engineering validation and platform-specific fixes;
9. hands-on validate target-platform behavior;
10. commit target-platform work there, reconcile docs, then integrate.

Temporary duplicate uncommitted working copies are acceptable. Git history is the authoritative record.

Long-term direction: CI/GitHub Actions for generic build/unit checks plus MCP-style or self-hosted real-machine validation for Windows/Linux/macOS and capture/NVENC/process-lifecycle tests.

## Agent working contract

- Read relevant canonical docs before architecture-sensitive work.
- Inspect existing code before implementing.
- Prefer the smallest clean change that satisfies the ticket.
- Preserve unrelated local changes.
- Do not push, merge, rebase, tag, or publish unless explicitly asked.
- Commit only when the current ticket/workflow explicitly permits it; otherwise leave a suggested commit message.
- Product, architecture, and UX decisions belong to the user in collaboration with ChatGPT. Do not silently broaden scope or substitute a new product decision.
- Explanation/brainstorming is not implementation permission.
- Engineering validation is not UX approval.
- Never claim native validation from a different OS.
- If target and current environment differ, state exactly what remains unvalidated.
- Do not tune sleeps/timeouts merely to hide lifecycle races without evidence.

## Reporting

Return implementation reports in one fenced block with:

- Implemented
- Behavior
- Engineering Validation
- Hands-on / UX Review
- Documentation Impact
- Notes
- Suggested Commit Message
- Status

Every implementation report assesses:

- `AGENTS.md`
- `docs/PGC-ARCHITECTURE.md`
- `docs/PGC-ROADMAP.md`
- `docs/PGC-DEV-NOTES.md`
- `docs/PGC-DECISIONS.md`
- `docs/PGC-VERSIONING.md`

Canonical docs are milestone-based shared project memory. Small tickets may accumulate documentation impact, but accepted working branches must be reconciled before merge into `master`, and `master` receives a broader review before milestone promotion to `main`.

## Product invariants

1. UI states describe observable reality, not implementation state.
2. `Playing` means decoded media has been confirmed, not merely process/port/RTSP connectivity.
3. Fresh connections never use reconnect language.
4. Reconnect states are only valid after a previously healthy Playing session.
5. Restored connectivity returns to `WaitingForStream` until media is confirmed.
6. Prefer explicit, debuggable failure states over silent fallback.
7. Host capture is demand-driven. Idle means no FFmpeg, capture device unopened, NVENC idle, no publisher traffic.
8. A few seconds of first-stream startup is acceptable for low idle footprint and reliable lifecycle behavior.
9. MediaMTX is relay/demand infrastructure; the native Host owns FFmpeg/capture lifecycle.
10. PGC is a broadcast/listener system; Client Cancel/Stop are local and should not tightly own Host sessions.
11. Preserve platform-native UX where useful; share core logic deliberately rather than forcing identical shells.
12. Host and Client remain separate applications.

## Logging delivery expectations

Every new user action, state transition, failure path, and background subsystem must decide what belongs in:

- **Normal** — important lifecycle/user-facing production diagnostics;
- **Debug** — structured event-driven reasoning/internal diagnostics;
- **Verbose** — Debug plus raw external provenance and deeper internal decision paths.

UTC/Zulu millisecond timestamps are canonical. Redirected/file output is plain text. Interactive terminal styling is presentation only.

Development-only smoke-test tools are encouraged when they reduce otherwise unverifiable behavior, remain contained, do not ship in production, and do not alter product behavior. They are engineering validation, never a substitute for hands-on UX acceptance.

## Current Client interaction contract

- Idle: `Search for Host`
- Busy/warm-up/recovery: `Cancel`
- Playing: `Stop Stream`
- Error: `Retry`
- Quit remains available
- initial launch has no focused control
- no default button
- Space / Return / Enter activate a deliberately focused button once
- held-key repeats must not chain through replacement controls
- focus follows the logical primary-action slot when that control is replaced
- if a focused control disappears without a replacement, focus clears rather than falling through to Quit
- `L` opens the logs folder
- no Client -> Host cancel/session coupling

## Current topology

```text
Console / AVR / capture device
-> Windows FFmpeg
-> SRT localhost
-> MediaMTX
-> RTSP/TCP LAN
-> Client / ffplay
-> Discord / local viewing
```

PowerShell/PID-file FFmpeg ownership is legacy/deprecated and must not be reintroduced without an explicit architecture decision.
