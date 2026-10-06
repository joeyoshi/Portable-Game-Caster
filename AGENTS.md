# Portable Game Caster — Agent Guide

Portable Game Caster (PGC) is a local-network gameplay streaming/capture system under the JoeYoshi banner.

Use "Portable Game Caster" in prose and user-facing text. "PGC" is fine in code identifiers, log provenance (`[PGC]`), file names, and document shorthand.

## Project identity

- Current license: `GPL-3.0-only`. Do not change `LICENSE` without an explicit product decision.
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

Working branches should preserve useful incremental history. A completed engineering ticket will often map cleanly to one focused commit after engineering validation. Small ticket commits may use concise one-line subjects. Richer messages belong at platform acceptance checkpoints, major architectural transitions, and feature integration.

A commit records an implementation checkpoint. It does not by itself mean UX, hardware behavior, or the entire feature has been accepted.

Agents may commit only when the current ticket or workflow explicitly permits it. Merely being on a feature branch is not permission.

## Validation and acceptance

Keep these concepts separate:

- **Implemented**: code exists.
- **Engineering Validated**: appropriate builds/tests/synthetic checks passed.
- **Awaiting Hands-on Validation / UX Validation**: engineering checks passed; human/native behavior still needs review.
- **UX Approved**: user-visible behavior has been hands-on accepted.
- **Done**: feature/platform scope is accepted and synchronized.

User-visible, hardware-dependent, and platform-specific work still requires hands-on/native validation before that platform or feature is considered accepted or merged into `master`.

If hands-on testing finds a defect after a ticket commit, fix it in a later commit rather than rewriting useful history.

## Branch-scope guardrails

A working branch should have one primary intent. Small adjacent changes may stay on that branch when they are directly required to implement, validate, diagnose, or make the primary feature usable.

Before broadening scope, ask whether the new work:

- is independently reviewable as a feature or fix;
- changes an unrelated subsystem or product area;
- deserves its own release-note or acceptance pass;
- is starting to dominate the branch diff or ticket sequence; or
- can be cleanly based on the current branch without blocking completion.

If several are true, propose a sibling or follow-on branch instead of silently expanding scope.

## Agent working contract

- Read relevant public documentation before architecture-sensitive work.
- Inspect existing code before implementing.
- Prefer the smallest clean change that satisfies the ticket.
- Preserve unrelated local changes.
- Do not push, merge, rebase, tag, release, or publish unless explicitly asked.
- Commit only when the current ticket or workflow explicitly permits it; otherwise leave a suggested commit message.
- Product, architecture, and UX decisions belong to the user in collaboration with the project maintainers. Do not silently broaden scope or substitute a new product decision.
- Explanation or brainstorming is not implementation permission.
- Engineering validation is not UX approval.
- Never claim native validation from a different OS.
- If target and current environment differ, state exactly what remains unvalidated.
- Do not tune sleeps/timeouts merely to hide lifecycle races without evidence.

## Reporting

Implementation reports should include:

- Implemented
- Behavior
- Engineering Validation
- Hands-on / UX Review
- Documentation Impact
- Notes
- Suggested Commit Message
- Status

Update public documentation when implementation changes user-visible behavior, architecture contracts, build requirements, protocol behavior, or contributor-facing invariants.

## Product invariants

1. UI states describe observable reality, not implementation state.
2. `Playing` means decoded media has been confirmed, not merely process/port/RTSP connectivity.
3. Fresh connections never use reconnect language.
4. Reconnect states are only valid after a previously healthy Playing session.
5. Restored connectivity returns to `WaitingForStream` until media is confirmed.
6. Prefer explicit, debuggable failure states over silent fallback.
7. Host capture is demand-driven. Idle means no FFmpeg, capture device unopened, encoder idle, and no publisher traffic.
8. A few seconds of first-stream startup is acceptable in exchange for low idle overhead and reliable lifecycle behavior.
9. MediaMTX is relay/demand infrastructure; the native Host owns FFmpeg/capture lifecycle.
10. PGC is a broadcast/listener system; Client Cancel and Stop are local actions and should not tightly own Host sessions.
11. Preserve platform-native UX where useful; share core logic deliberately rather than forcing identical shells.
12. Host and Client remain separate applications.
13. Core discovery, control, and media paths are LAN-local; Internet access must not be required for basic operation.

## Logging expectations

Every new user action, state transition, failure path, and background subsystem must decide what belongs in:

- **Normal** — important lifecycle/user-facing production diagnostics;
- **Debug** — structured event-driven reasoning/internal diagnostics;
- **Verbose** — Debug plus raw external provenance and deeper internal decision paths.

UTC/Zulu millisecond timestamps are canonical. Redirected/file output is plain text. Interactive terminal styling is presentation only.

Development-only test tools are encouraged when they reduce otherwise unverifiable behavior, remain contained, do not alter production behavior, and are excluded from release builds where appropriate. They are engineering validation, never a substitute for hands-on acceptance.

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
capture device
-> Windows FFmpeg
-> SRT localhost
-> MediaMTX
-> RTSP/TCP LAN
-> Portable Game Caster Client
```

PowerShell/PID-file FFmpeg ownership is legacy/deprecated and must not be reintroduced without an explicit architecture decision.
