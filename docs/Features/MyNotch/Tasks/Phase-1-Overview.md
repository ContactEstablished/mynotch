# Phase 1 kickoff — Typing, global shortcut, app search

Status: **done 2026-10-15** (Task 1-1 `4bfe42e`, Task 1-2 `8251ab7`, Task 1-3 `27e6323`). All decisions below are resolved; P1-D1 was amended at the end. The kickoff-time description follows. Baseline: `63f6951` ("Add Time tab, notch settings, sliding, and app reordering") on `main`; the only change in the working tree is the untracked `docs/` folder (the roadmap and these documents).

## Source Of Truth

- [roadmap](../roadmap.md) — Phase 1 section, gates G1–G7, decision D6 (resolved below as P1-D1).
- [HANDOFF.md](../../../../HANDOFF.md) — current state, the verification method (scripted input + screenshots), tone, known rough edges.
- These task/spec pairs. They are the HOW; where they differ from the roadmap, the differences are listed under "Roadmap amendments" for the coordinator to record.
- Code under change: `src/app.rs`, `src/text.rs` (new), `src/clipboard.rs`, `src/gfx.rs`, `src/main.rs`, `src/widgets/mod.rs`, `src/widgets/apps.rs`, `src/hotkey.rs` (new), `src/settings.rs`, `src/settings_panel.rs`.

## Goal

Give the notch a keyboard. A search bar in the tab row filters installed apps as you type; arrow keys move the highlight and Enter launches. A global shortcut (default Ctrl+Alt+Space) opens that search from any app, and the shortcut can be re-recorded in the Settings tab. The notch takes keyboard focus only after an explicit click or the shortcut, and gives it back (G3). Phase 4 will reuse the text field and the search bar (quick-add).

## Tasks

The three tasks are sequential: each starts from the previous one's finished, committed state. `src/app.rs` and `src/main.rs` are shared, but never in parallel.

| Task | Title | Owns | Depends on |
| --- | --- | --- | --- |
| [1-1](Task-1-1.md) / [spec](../ImplementationSpecs/ImplementationSpec-1-1.md) | Typing foundation and the tab-row search bar | new `src/text.rs`; `src/clipboard.rs` (`read_text`/`write_text`); `src/gfx.rs` (`glyph::SEARCH`); `src/app.rs`; `src/main.rs` (`mod text`) | — |
| [1-2](Task-1-2.md) / [spec](../ImplementationSpecs/ImplementationSpec-1-2.md) | App search results | `src/widgets/mod.rs` (search hooks, `SearchKey`, `SearchAction`); `src/widgets/apps.rs` (results, ranking + tests); `src/app.rs` (routing only) | 1-1 |
| [1-3](Task-1-3.md) / [spec](../ImplementationSpecs/ImplementationSpec-1-3.md) | Global shortcut and the Settings recorder | new `src/hotkey.rs`; `src/settings.rs`; `src/settings_panel.rs`; `src/app.rs`; `src/main.rs` (`mod hotkey`); `HANDOFF.md` | 1-2 |

What each delivers:

- **1-1.** A pure, unit-tested single-line editing model (`text::TextField`) and its drawing; the shell taking and returning keyboard focus; a magnifier button carved out of the tab row's empty stretch; clicking it turns the stretch into a search field. While searching, the content area shows a dim "Type to search apps" hint.
- **1-2.** Search hooks on the `Widget` trait; the Apps widget ranks installed apps and shows them as tiles; Up/Down/Tab move the highlight; Enter or a click launches.
- **1-3.** `RegisterHotKey` with the saved shortcut; the shortcut toggles search from anywhere, including over a fullscreen app; a fourth Settings row records a new shortcut; HANDOFF.md updated; release exe rebuilt and relaunched.

## Resolved decisions

| ID | Decision | Status |
| --- | --- | --- |
| P1-D1 (roadmap D6) | Global shortcut default **Ctrl+Alt+Space** (amended 2026-10-15 to **Ctrl+Alt+K** by the user: Claude desktop owns Ctrl+Alt+Space on the dev PC). Changed by **recording** it in a new Settings-tab "Shortcut" row: click the chip, press the keys. Backspace or Delete while recording clears it (no shortcut). Esc cancels. A conflict (`RegisterHotKey` fails) is shown on that row ("Taken by another app") and is never fatal. Saved as `hotkey=` in settings.ini; `hotkey=` with an empty value means none. | RESOLVED by user, 2026-10-15 |
| P1-D2 | Search lives in a **bar in the tab row**. A magnifier button sits at the right end of the empty stretch, carved out of `handle_rect` so the ↔ cursor and slide-drag don't apply to it. Clicking it, or the shortcut, turns the whole stretch into a search field drawn as an outlined tab joining the panel (as the active tab does today); while searching no widget tab is drawn active and the baseline opens under the field. Results fill the content area. The stretch can't be dragged while searching. Phase 4 reuses this bar for quick-add. | RESOLVED by user, 2026-10-15 |
| P1-D3 | Esc closes search, clears it, and gives focus back to the previous window. | RESOLVED by coordinator, 2026-10-15 (user informed) |
| P1-D4 | Clicking a tab (any tab, including Home and Settings) while searching closes search, gives focus back, and selects that tab. | RESOLVED by coordinator, 2026-10-15 (user informed) |
| P1-D5 | An empty query shows the pinned apps (installed ones, in pinned order), so the shortcut doubles as a quick launcher. | RESOLVED by coordinator, 2026-10-15 (user informed) |
| P1-D6 | Ranking, case-insensitive: tier 0 the name starts with the query; 1 a word in the name starts with it; 2 the initials start with it ("vsc" → Visual Studio Code); 3 it appears anywhere. Ties: pinned first, then shorter name, then alphabetical. | RESOLVED by coordinator, 2026-10-15 (user informed) |
| P1-D7 | Down/Up (and Tab/Shift+Tab) move the result highlight in reading order. Left/Right stay with the caret. | RESOLVED by coordinator, 2026-10-15 (user informed) |
| P1-D8 | Enter, or clicking a result, launches it and closes search **without** restoring the previous window, so the launched app gets the foreground. | RESOLVED by coordinator, 2026-10-15 (user informed) |
| P1-D9 | Copies made from the field go into Clipboard history like any other copy. | RESOLVED by coordinator, 2026-10-15 (user informed) |
| P1-D10 | The shortcut also works while the notch is hidden over a fullscreen app: the notch reappears with search open; Esc returns focus to that app, which then hides the notch again. | RESOLVED by coordinator, 2026-10-15 (user informed) |
| P1-D11 | Pressing the shortcut while search is open closes it (toggle), like Esc. | RESOLVED by coordinator, 2026-10-15 (user informed) |
| P1-D13 | A recordable shortcut needs the Win key, or at least two of Ctrl/Alt/Shift, plus one key from the key table. This refuses Ctrl+C, Alt+F4 and other single-modifier app shortcuts, which a global registration would steal from every app. | RESOLVED by coordinator, 2026-10-15 (user informed) |
| P1-D12 | Roadmap Phase 1 goal 2 ("keyboard hooks for widgets") is narrowed: Phase 1 adds keyboard input to shell-owned fields (the search bar and the shortcut recorder) plus search hooks on the `Widget` trait. Widget-hosted text fields arrive with Phase 4. | RESOLVED by coordinator, 2026-10-15 (user informed); roadmap amendment below |

## Gates that apply

- **G1 — run, don't just compile.** Each task is verified in the real app with scripted input (`SendInput`) and screenshots, from PowerShell scripts kept in the implementer's session scratchpad, never in the repo. Back up `%APPDATA%\MyNotch\settings.ini` first and restore it after; back up and restore the clipboard text for clipboard tests. Restore the cursor position after mouse tests.
- **G2.** `cargo clippy --all-targets` with no warnings and `cargo test` green (39 tests at baseline, plus each task's new tests).
- **G3 — focus discipline.** Hovering or expanding never takes focus. The notch takes keyboard focus only after a click on the magnifier, the field or the shortcut chip, or the global shortcut. It gives focus back to the previous window on Esc, on the shortcut toggle and on a tab click. Interpretations, recorded as amendments: on launch (P1-D8) the launched app gets the foreground instead; on a click elsewhere the window clicked keeps it; "collapse" cannot happen while the notch holds the keyboard, because holding it keeps the notch open.
- **G7.** Commit straight to `main`. The user's standing workflow is one plain-English commit per finished task (the `commit-and-push` skill exists), but commit only when the session's workflow calls for it. A running release exe locks `target\release`; quit it before release builds.

## Risks

- **Windows' foreground lock.** A process may take the foreground only in limited cases, for example when it received the last input event. Precedent: the right-click menu calls `SetForegroundWindow` right after a user click (`src/app.rs:1041`), and the colour picker does the same for its dialog (`src/settings_panel.rs:196`). The window is created `WS_EX_NOACTIVATE` (`src/app.rs:370`); that blocks activation by clicks, not explicit `SetForegroundWindow`. Phase 1 calls it after a click (Task 1-1) and after `WM_HOTKEY` (Task 1-3). Mitigation: `begin_typing` checks that the notch really became the foreground window and backs out if not, and `tick` ends typing if the foreground moves away without a `WM_ACTIVATE` arriving. Both paths must be confirmed on the real machine (G1); if focus does not arrive, stop and report rather than adding workarounds such as `AttachThreadInput` or synthetic Alt presses.
- **Re-entrancy.** `with_app` uses `try_borrow_mut` and silently drops any message that arrives while `APP` is borrowed (`src/app.rs:991-999`). `SetForegroundWindow` sends `WM_ACTIVATE`/`WM_SETFOCUS` synchronously, so those messages are dropped when they come from our own calls. Rule: put all state in its final form *before* any focus call, and never rely on a re-entrant message being handled.
- **WM_CHAR surrogates.** Characters outside the BMP (emoji from Win+., some CJK) arrive as two `WM_CHAR` messages. `text::Utf16Joiner` pairs them; unpaired halves are dropped.
- **AltGr layouts.** AltGr is reported as Ctrl+Alt. AltGr+C/V/X/A types characters on some layouts (Polish ć, ą, ź), so Ctrl+Alt+key must not count as an editing command.
- **Alt+F4 while typing.** Alt+F4 on the focused notch would reach `DefWindowProcW` and quit MyNotch. While the notch holds the keyboard, Alt+F4 acts as Esc.
- **RegisterHotKey conflicts.** Another app may own Ctrl+Alt+Space; Windows reserves many Win+ combinations and F12. A failure is shown on the Settings row and is never fatal. Some Win+ combinations are handled by the shell before MyNotch sees them while recording; those can't be recorded.
- **Settings tab crowding.** Three rows of 56 px fill 172 of the content area's 176 px (`src/settings_panel.rs:41-42`, `CONTENT` at `src/app.rs:69`). A fourth row needs `ROW_H` = 43 (4 + 4 × 43 = 176). Check the swatch selection rings and the slider's hit area in screenshots.
- **Tab-row handle shrinks.** Carving the magnifier (44 px + 4 px gap) out of the stretch takes the drag handle from x 312–616 to x 312–568, and moves the grip's centre from 464 to 440. HANDOFF's regression-script coordinates need that update.

## Roadmap amendments (for the coordinator to record)

1. D6 → RESOLVED (P1-D1).
2. Phase 1 goal 2 narrowed per P1-D12: keyboard for shell-owned fields plus `Widget` search hooks; widget-hosted text fields move to Phase 4.
3. Phase 1 goal 5 ("Apps search"): search is a tab-row bar (P1-D2), not a filter inside the Apps picker. The picker itself is unchanged.
4. G3 interpretation for Phase 1: launch hands the foreground to the launched app (P1-D8); a click elsewhere leaves it where the user clicked; the notch cannot collapse while it holds the keyboard.

## The running release instance

A release build is running now from `target\release\mynotch.exe` (started 2026-10-15). The single-instance mutex (`src/desktop.rs:20-28`) makes `cargo run` or a second exe exit at once. Before any runtime test, quit it: right-click → Quit, or find the window class `MyNotchWindow` (`src/app.rs:358`) and post `WM_CLOSE`. A debug `cargo build` is not blocked by it, but `cargo build --release` is.

At the end of Task 1-1 or 1-2, if the session stops there, relaunch the existing `target\release\mynotch.exe` so the user still has a notch. Task 1-3 rebuilds the release exe after the final checks and relaunches it, leaving the user on the new version.

## Verification approach (all tasks)

- Scripts live in the implementer's session scratchpad and use `SendInput` for real mouse and keyboard input, `GetForegroundWindow` for focus checks, and screenshots of the notch region (as HANDOFF describes).
- Focus tests use a throwaway "previous window": a WinForms form titled "MyNotch focus test", started in a separate PowerShell process, with a text box that logs its contents to a scratch file. The form is closed at the end.
- Order at the end of every runtime session: quit the dev MyNotch, restore settings.ini (and the clipboard), then relaunch the release exe.

## Kickoff validation

The line numbers cited in these documents were re-read against `63f6951` on 2026-10-15. This is documentation only: no build, test or runtime check was run for Phase 1.
