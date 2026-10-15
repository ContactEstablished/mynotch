# MyNotch roadmap — typing, Live Status, Signal integration, indicators

Status: **drafted 2026-10-15.** Phase 1 kicked off 2026-10-15 (task docs written; no code yet). Baseline: `63f6951` ("Add Time tab, notch settings, sliding, and app reordering") on `main`.

## Purpose

MyNotch is our native Rust (`windows` 0.62 + Direct2D) take on BuildNotch: a glass "notch" at the top of the screen that opens on hover and hosts widgets. HANDOFF.md's Milestones 1–4 are done. This effort adds four things:

1. **Typing.** The notch cannot take keyboard input today. Phase 1 adds a text field, a global shortcut and search in the Apps picker.
2. **Live Status.** Scripts and tools (Claude Code hooks first) can push a short status line into the notch.
3. **Signal integration.** Meetings, tasks and notes come from Signal, the user's own planner app, which already aggregates all of the user's calendars. Signal gains a small local API; MyNotch becomes a client of it.
4. **Indicators.** Mic/camera in use, volume and audio output, battery.

Prime contracts for the whole effort:

- **The notch never steals focus.** It takes the keyboard only after an explicit user action and gives it back (G3).
- **Signal owns its data.** MyNotch never syncs calendars (D1) and never touches signal.db (D2).
- **Everything stays local.** Nothing listens on a network port (G5).

Numbering: the phases below are numbered Phase 1–5 and follow HANDOFF.md's Milestones 1–4. "Phase N" in this folder never means "Milestone N", and it is unrelated to Signal's own milestone numbers (see D10).

## Source Of Truth

- [HANDOFF.md](../../../HANDOFF.md) — current state, the running decision log, the verification method (scripted input + screenshots), known rough edges and the feature backlog.
- This roadmap — WHAT and WHY for Phases 1–5. HOW lives beside it in `Tasks/` and `ImplementationSpecs/`, created per phase by phase-kickoff.
- MyNotch code under change: `src/main.rs`, `src/app.rs` (window, focus, input routing), `src/widgets/mod.rs` (Widget trait, collapsed readout), `src/widgets/apps.rs` (picker), `src/settings.rs`, `src/desktop.rs` (single instance), `Cargo.toml`.
- Signal (`C:\Projects\ContactEstablished\Signal`): `AGENTS.md` (binding rules for any Signal change), `PLAN.md` (its product plan, including the unbuilt Notes view), `docs/Features/Signal/` (the layout this folder mirrors), `src-tauri/` (the future home of the API).
- Cross-repo protocol doc — does not exist yet. Phase 3 publishes it; both repos link to it (D4, D10).

## Verified Ground Facts

All facts verified 2026-10-15. Paths are relative to each repo's root.

### MyNotch (`C:\Projects\ContactEstablished\MyNotch`)

| Fact | Evidence | Verified |
| --- | --- | --- |
| No `docs/` folder existed before this roadmap; HANDOFF.md is the running state and decision log. | Repo tree at `63f6951` | 2026-10-15 |
| `cargo test`: 39 passing; `cargo clippy --all-targets`: no warnings. | Both run at `63f6951` | 2026-10-15 |
| The exe is GUI-subsystem: no console, so a CLI subcommand can't print to the terminal without attaching to the parent console, and shells may not wait for it. | `src/main.rs:1` (`#![windows_subsystem = "windows"]`) | 2026-10-15 |
| The notch window never takes focus, so there is no keyboard input today. It is created `WS_EX_LAYERED \| WS_EX_TOPMOST \| WS_EX_TOOLWINDOW \| WS_EX_NOACTIVATE` and answers `WM_MOUSEACTIVATE` with `MA_NOACTIVATE`. | `src/app.rs:370`, `src/app.rs:1154` | 2026-10-15 |
| The right-click menu already takes the foreground briefly with `SetForegroundWindow` — precedent for a short, user-initiated focus grab. | `src/app.rs:1041` | 2026-10-15 |
| The `Widget` trait has mouse hooks (click, press, drag, release, wheel), load/store, take_attention, compact and Home card hooks. It has no keyboard or text hooks. | `src/widgets/mod.rs:79-150` | 2026-10-15 |
| The collapsed readout is `Compact { priority, text, live }`; the highest priority wins. Current priorities: focus running 20, media playing 10, stopwatch running 8, focus paused 5, stopwatch paused 4 (System's CPU/RAM fallback is 1). | `src/widgets/mod.rs:70-72`; `focus.rs:146,149`; `media.rs:168`; `stopwatch.rs:64,66`; `system.rs:91` | 2026-10-15 |
| Single instance uses the named mutex `Local\MyNotch.SingleInstance`; a second copy currently just exits. | `src/desktop.rs:21-29` | 2026-10-15 |
| The Apps picker (`AppsWidget.picking`) shows every installed app, sorted by name, in a scrollable grid. There is no search. | `src/widgets/apps.rs:85-100` | 2026-10-15 |
| Settings are a key=value file at `%APPDATA%\MyNotch\settings.ini`. Keys: `pinned`, `hide_in_fullscreen`, `tab`, `focus_minutes`, `apps`, `color`, `opacity`, `position`. | `src/settings.rs:76-98` | 2026-10-15 |
| No JSON/serde dependency yet; the only dependencies are `sysinfo`, `windows-numerics` and `windows`. | `Cargo.toml` | 2026-10-15 |

### Signal (`C:\Projects\ContactEstablished\Signal`)

| Fact | Evidence | Verified |
| --- | --- | --- |
| Signal is the user's own Tauri 2 + Svelte 5 + SQLite app, version 0.1.7. It already aggregates all of the user's calendars' meetings, plus projects, tasks and timers. | `src-tauri/tauri.conf.json:3`; user, 2026-10-15 | 2026-10-15 |
| No external API or integration exists in Signal today (the only socket is a mock server in a test). | `src-tauri/src/` search; `src-tauri/src/intake/tests.rs:382` | 2026-10-15 |
| Tokio features are only `time`, `process`, `fs`, `io-util`, `sync`. A named-pipe or socket server needs `net`. | `src-tauri/Cargo.toml:23` | 2026-10-15 |
| The SQLite pool exists only after the frontend opens the database; until then `db::pool` returns "The local database is not open yet." Clock and agenda initialisation also run from the frontend bootstrap. An API must therefore answer "not ready" until Signal has finished starting. | `src-tauri/src/db.rs:94-102` | 2026-10-15 |
| Meeting occurrence queries reject ranges over 8 days ("Meeting queries require a bounded day or week."). "Next meeting" must scan forward in windows of 8 days or less. | `src-tauri/src/agenda/recurrence.rs:215` | 2026-10-15 |
| Task creation is reusable (`tasks::create` / `create_in`); updates (`update`) use optimistic revisions; mutations elsewhere use client-chosen request_id idempotency receipts (`agenda_requests`, `timer_requests`, `planner_requests`). | `src-tauri/src/workspace/tasks.rs:236,434,440` | 2026-10-15 |
| There is no notes table (migrations 0001–0009). Notes exist only as task/meeting markdown fields. PLAN.md lists a Notes sub-view that is not built yet. | `src-tauri/migrations/`; `PLAN.md:181,275` | 2026-10-15 |
| Closing Signal's window quits it unless `close_to_tray` is on; it defaults to false. | `src-tauri/migrations/0001_initial.sql:57`; `src-tauri/src/window.rs:134-147` | 2026-10-15 |
| Identifiers: dev `dev.contactestablished.signal`, personal `com.contactestablished.signal`. Both can run at once, so any endpoint name must differ per identifier. | `src-tauri/tauri.conf.json:3`; `src-tauri/tauri.personal.conf.json:3` | 2026-10-15 |
| Signal's rules: changes stay in development until the user explicitly asks to push live; never use the personal config or personal data for development or tests; back up the whole roaming app-data folder before an authorized upgrade. | `AGENTS.md` | 2026-10-15 |

## Global Decisions & Gates

### Decisions

| ID | Decision | Status | Why |
| --- | --- | --- | --- |
| D1 | Meetings, tasks and notes come from Signal through a local API added to Signal. MyNotch never syncs calendars itself (no OAuth, no ICS). | RESOLVED (2026-10-15, data comes from Signal) | The user has several calendars and Signal already aggregates them. |
| D2 | The integration is a local API owned by Signal, not MyNotch reading signal.db. | RESOLVED (2026-10-15, Signal-owned API) | Keeps Signal's validation, revisions and idempotency in charge. Direct reads couple MyNotch to the schema and can't write safely. |
| D3 | Notes live in a new Signal notes table. | RESOLVED (2026-10-15, new table in Signal) | Signal has no notes table; its own planned Notes view can show them. |
| D4 | Transport: a per-user Windows named pipe whose name derives from Signal's identifier; JSON messages carrying a protocol version; current-user-only access; no TCP port. | PROPOSED — confirm at Phase 3 kickoff | Dev and personal builds can run at once, so names must differ. Satisfies G5. |
| D5 | Live Status transport: `mynotch status …` runs the same exe, which forwards the message to the running notch and exits. No server, no port. | PROPOSED — confirm at Phase 2 kickoff | Reuses the single-instance exe; nothing new listens. |
| D6 | Global shortcut: default Ctrl+Alt+Space, changeable, saved in settings.ini. A conflict is reported, not fatal. | RESOLVED (2026-10-15, user: default Ctrl+Alt+Space, changed by recording it in a Settings-tab row; see Phase-1-Overview P1-D1, P1-D13) | Another app may already own the combination. |
| D7 | Which Signal project receives tasks added from the notch. Proposed: a default project chosen in the notch's Settings, with a chip to override it when adding. | OPEN — needed by Phase 4 | Tasks added from the notch need a home in Signal. |
| D8 | Keeping Signal available: turn on close-to-tray, and possibly add Start with Windows to Signal. | OPEN — needed by Phase 4 | Meetings only show while Signal runs, and closing its window quits it by default. |
| D9 | Does Phase 3 include a minimal Notes view inside Signal, or do notch-created notes wait for Signal's planned Notes view? | OPEN — decide at Phase 3 kickoff | Without a view, notes made in the notch are invisible inside Signal. |
| D10 | Phase 3 is planned and tracked in the Signal repo under its own docs and AGENTS.md rules. This roadmap records only the cross-repo contract and the dependency. | PROPOSED | Signal has its own docs, milestone numbering and release rules. |
| D11 | Notification mirroring is dropped. | RESOLVED (2026-10-15, dropped) | Reading Windows notifications likely requires package identity (MSIX), which MyNotch doesn't have. |

### Gates

| ID | Gate | Applies to |
| --- | --- | --- |
| G1 | **Run, don't just compile.** Every phase is verified in the real app with scripted input and screenshots, as HANDOFF's verification does. The user's settings.ini (and the clipboard, when touched) is backed up first and restored after. | All phases |
| G2 | `cargo clippy --all-targets` with no warnings and `cargo test` green before a phase is called done. | All MyNotch phases |
| G3 | **Focus discipline.** Hovering or expanding never takes focus. The notch takes keyboard focus only after an explicit user action (clicking a text field or pressing the shortcut) and gives it back on Esc, submit, collapse or a click elsewhere. The previously active window gets focus back. | Phases 1, 2, 4 |
| G4 | **Signal safety.** All Signal work uses the dev build and dev data only. The personal install is touched only when the user explicitly asks, after a full backup of `%APPDATA%\com.contactestablished.signal`. | Phases 3, 4 |
| G5 | **Local only.** No API listens on a network port. Pipes refuse remote clients and other users. | Phases 2, 3, 4 |
| G6 | **Graceful absence.** When Signal isn't running or isn't ready, MyNotch says so plainly and keeps working. | Phase 4 |
| G7 | Commit straight to `main` (the user's workflow; no milestone branches). A running release exe locks `target\release`; quit it before release builds. | All MyNotch phases |

## Phases

| Phase | Repo | Status | Depends on |
| --- | --- | --- | --- |
| 1 — Typing, global shortcut, app search | MyNotch | Kicked off 2026-10-15 — Tasks 1-1…1-3 ready | — |
| 2 — Live Status | MyNotch | Provisional | — |
| 3 — Signal local API v1 | Signal | Provisional | — (can run in parallel with Phases 1–2) |
| 4 — Agenda, Tasks and Notes in the notch | MyNotch | Provisional | Phases 1 and 3 |
| 5 — Indicators | MyNotch | Provisional | — |

### Phase 1 — Typing, global shortcut, app search

Status: **Kicked off 2026-10-15; not started.** Repo: MyNotch only. D6 resolved at kickoff.

Docs: [Phase-1-Overview](Tasks/Phase-1-Overview.md) (decisions P1-D1…P1-D13), [Task 1-1](Tasks/Task-1-1.md), [Task 1-2](Tasks/Task-1-2.md), [Task 1-3](Tasks/Task-1-3.md), with their [ImplementationSpecs](ImplementationSpecs/).

Amendments at kickoff (2026-10-15; the goals below are kept as originally written):

- Goal 2 is narrowed: Phase 1 gives the keyboard to shell-owned fields (the search bar and the shortcut recorder) and adds search hooks to the `Widget` trait. Text fields hosted inside widgets move to Phase 4 (P1-D12).
- Goal 5: search is a bar in the tab row, opened by a magnifier button or the shortcut, with results filling the panel; the Apps picker itself is unchanged (P1-D2, user's choice).
- G3 as applied in Phase 1: launching an app hands the foreground to that app; a click elsewhere leaves focus where the user clicked; the notch can't collapse while it holds the keyboard.

Why: agenda, tasks and notes (Phase 4) all need text input, and the notch has none. App search is the smallest real use of typing and closes HANDOFF's "no search in the picker" rough edge.

Goals:

1. **Text field.** A single-line field the notch can host: caret, insert and delete, cursor keys, Home/End, select-all, cut/copy/paste, Enter submits, Esc cancels.
2. **Keyboard hooks for widgets.** A focused widget receives keys and typed characters, the way widgets receive mouse events today.
3. **Focus rules** exactly as G3.
4. **Global shortcut** (D6). From any foreground app it opens the notch with the Apps search focused.
5. **Apps search.** Typing filters installed apps by name. Arrow keys move the selection. Enter launches the highlighted match.

Non-goals:

- Multi-line editing (Phase 4 needs it for notes and adds it then).
- IME composition UI beyond receiving committed characters.
- Rescanning newly installed apps (they still appear after a restart).

Exit criteria:

- The text-editing model and the search matching are unit-tested.
- Real-app verification (G1) of: typing and editing in the field; the shortcut pressed while another app is in the foreground; focus returning to the previous window on Esc, submit, collapse and click-elsewhere; hover still never stealing focus.
- G2 green; HANDOFF.md updated with the new state; committed to `main` (G7).

Risk to check at kickoff: Windows restricts which processes may take the foreground. The menu's brief grab is the only precedent, so kickoff must confirm that focus actually arrives after the shortcut and after a click, and returns afterwards, on the real machine.

### Provisional outline — NOT yet authoritative

Phases 2–5 are a sketch. Each becomes authoritative only at its own kickoff, which may re-plan it.

#### Phase 2 — Live Status (MyNotch)

Decision to confirm at kickoff: D5.

- Command: `mynotch status "<text>" [--id] [--kind info|success|error|attention] [--progress 0..1] [--ttl]`, and `--clear <id>` to remove one.
- Statuses show in the collapsed readout. `attention` ranks above everything; where the other kinds rank against focus (20) and media (10) is decided at kickoff. Home gets a card with the current stack.
- `attention` pops the notch open, like a finished focus session.
- Statuses live in memory only and are lost on restart.
- Documented recipes for Claude Code hooks (Notification → "Claude needs input", Stop → "Claude finished") and for build/test scripts.
- Must handle the GUI-subsystem console problem: usage and errors are visible when run from a terminal.
- When the notch isn't running, the command fails with a clear message and exit code.

#### Phase 3 — Signal local API v1 (Signal repo)

Tracked in the Signal repo (D10). Can run in parallel with Phases 1–2. Decisions: D4 and D9 at kickoff.

- Endpoint per D4, with distinct names for the dev and personal identifiers.
- Answers "not ready" until Signal has finished starting (database open, clock and agenda initialised).
- Reads: status/version, next meeting (current or upcoming, scanning windows of 8 days or less), today's agenda (the same data as Signal's Today), projects, open tasks, notes, running timers.
- Writes: create task, complete task, create/edit note — all through Signal's existing validation, revisions and request-id idempotency.
- Signal's UI refreshes after an API write.
- New notes table (D3); a Notes view only if D9 says so.
- Dev build and dev data only (G4). Signal's own gates from its AGENTS.md apply on top of this roadmap's.
- Publishes a short protocol doc that both repos reference.

#### Phase 4 — Agenda, Tasks and Notes in the notch (MyNotch)

Needs Phases 1 and 3. Decisions D7 and D8 must be resolved first.

- A Signal client in MyNotch (MyNotch's first JSON use, unless Phase 2 already added it).
- The collapsed readout shows the next meeting ("Standup in 12 min"), with priority rules against focus, media and Live Status.
- A Join button for meetings that have a link.
- Today's meetings.
- Open tasks, with add and complete; new tasks go to the D7 project.
- A notes list and note editing, which needs multi-line text.
- Quick-add from the global shortcut.
- The D7 default project is chosen in the notch's Settings.
- G6: with Signal closed or still starting, every Signal-backed view says so and the rest of the notch keeps working.

#### Phase 5 — Indicators (MyNotch)

- A mic/camera in-use dot, read from Windows' capability-access records.
- A volume pop-up when the volume changes, plus an audio output switcher.
- Battery: the laptop's, and Bluetooth devices' where Windows reports it.
- To confirm at kickoff: whether Windows offers a supported way to switch the default audio output device.

## Out Of Scope

For the whole effort:

- Calendar sync, OAuth or ICS in MyNotch (D1).
- Notification mirroring (D11).
- MyNotch reading or writing signal.db directly (D2).
- MSIX packaging.
- Multi-monitor support.
- Any network-exposed API (G5).
- Other BuildNotch backlog items — AI-app tracker, GitHub activity, timer themes, snippet playground. They stay in HANDOFF's backlog.
