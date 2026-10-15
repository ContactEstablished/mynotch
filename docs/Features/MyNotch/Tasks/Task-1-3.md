# Task 1-3 — Global shortcut and the Settings recorder

Status: **ready; not started.** Depends on Task 1-2. Paired spec: [ImplementationSpec-1-3](../ImplementationSpecs/ImplementationSpec-1-3.md). Closes Phase 1.

## Source Of Truth

- [Phase 1 overview](Phase-1-Overview.md): decisions P1-D1, P1-D10, P1-D11; gates G1, G2, G3, G7; risks (foreground lock, RegisterHotKey conflicts, Settings crowding); the release-instance note.
- [roadmap](../roadmap.md): Phase 1 goal 4, decision D6, the exit criteria. [HANDOFF.md](../../../../HANDOFF.md): updated by this task.
- [ImplementationSpec-1-3](../ImplementationSpecs/ImplementationSpec-1-3.md), plus the names from Specs [1-1](../ImplementationSpecs/ImplementationSpec-1-1.md) and [1-2](../ImplementationSpecs/ImplementationSpec-1-2.md).

## Initial Starting Point

Expected after Tasks 1-1 and 1-2 (confirm before starting):

- The search bar works end to end: `open_search` takes the keyboard and shows the pinned apps; Enter launches; `end_typing(restore)` gives focus back. `Owner` has one variant, `Search`. `cargo test`: 58 passing.
- `src/app.rs` line numbers have moved since `63f6951`; find code by function name.

Unchanged since `63f6951` (verified 2026-10-15):

- `src/settings.rs`: `Settings` at 13-30 (keys `pinned`, `hide_in_fullscreen`, `tab`, `focus_minutes`, `apps`, `color`, `opacity`, `position`); `Default` at 32-45; `parse` at 65-109 (the key `match` at 75-106); `serialize` at 111-126; tests at 133-181 (`round_trips` at 138-155 lists every field explicitly).
- `src/settings_panel.rs`: `ROW_H = 56.0` at 41 and `ROW_Y = 4.0` at 42, so three rows fill 172 of the content area's 176 px (`CONTENT`, `src/app.rs:69`); `CHIP` at 50; `Hit` at 52-60; `row_centre` at 66-68; `center_chip` at 82-84; `hit` at 99-107; `draw` at 109-158 (Position row at 149-157); tests at 251-275.
- `src/app.rs` (baseline numbers): `run` at 349-402 (`App::new` at 384); the fullscreen poll in `tick` at 583-594 and the early return while hidden at 607-609; the Settings branch of `on_press` at 680-697; `settings_panel::draw` called at 853; `WM_DESTROY` at 1162-1166. No `WM_HOTKEY` handling. `desktop::foreground_is_fullscreen` ignores MyNotch's own windows (`src/desktop.rs:84`).
- `Cargo.toml` line 9 already enables `Win32_UI_Input_KeyboardAndMouse` (`RegisterHotKey`, `UnregisterHotKey`, `GetKeyState`).
- `src/main.rs:3-15` lists the modules; there is no `hotkey` module.

## Goal

Register a global shortcut (default Ctrl+Alt+Space) that toggles the search bar from any app, including over a fullscreen app (the notch reappears; Esc returns focus to that app, which hides the notch again). Add a fourth Settings row, "Shortcut", whose chip records a new combination: click it, press the keys. Backspace or Delete clears the shortcut, Esc cancels, and a combination Windows won't register shows "Taken by another app" without breaking anything. Save it as `hotkey=` in settings.ini. Finish the phase: update HANDOFF.md, rebuild the release exe and leave it running.

## Exact Scope

Create:

- `src/hotkey.rs` — `Hotkey` (parse, `Display`, validation, modifier bits), `is_modifier`, the key-name table, unit tests.

Edit:

- `src/main.rs` — `mod hotkey;`.
- `src/settings.rs` — `hotkey: Option<Hotkey>`, default, parse, serialize, tests.
- `src/settings_panel.rs` — `ROW_H` 43, `Hit::Shortcut`, `ShortcutRow`, the Shortcut row, tests.
- `src/app.rs` — `Owner::Shortcut`, `HOTKEY_ID`, `hotkey_taken`, `register_hotkey`, `on_hotkey`, `unhide`, `start_recording`, `record_key`, the `WM_HOTKEY`/`WM_SYSCHAR`/`WM_SYSKEYUP` arms, unregistering in `WM_DESTROY`, recorder routing in `on_press`/`on_key`, and passing `ShortcutRow` to `settings_panel::draw`.
- `HANDOFF.md` — the phase-end state update (spec § 7).

Nothing else.

## Non-Goals

- No new search behaviour, ranking or result drawing (Tasks 1-1/1-2), except what the shortcut needs to open search.
- No low-level keyboard hook; `RegisterHotKey` only. No "press the shortcut twice" or chord sequences.
- No live preview of held modifiers while recording.
- No roadmap edits; the coordinator records the amendments listed in the overview.
- No Start-with-Windows, tray-menu or single-instance changes.
- Do not revert or commit unrelated changes. `docs/` is untracked at kickoff (the roadmap and these Phase 1 docs): leave it alone, and don't fold it into the code commit unless the user asks.

## Dependencies

Tasks 1-1 and 1-2, finished and verified.

## Step-by-step Work

1. Write `src/hotkey.rs` with its tests; add `mod hotkey;` to `src/main.rs`.
2. Add `hotkey` to `Settings` (default, parse, serialize) and update and extend its tests.
3. `src/settings_panel.rs`: shrink `ROW_H` to 43, add the Shortcut row, `Hit::Shortcut`, `ShortcutRow`, and tests.
4. `src/app.rs`: register the shortcut at startup and after every change; handle `WM_HOTKEY` (toggle; unhide over fullscreen); add the recorder (`Owner::Shortcut`, unregister while recording, read modifiers with `GetKeyState`, ignore bare modifiers, swallow `WM_SYSKEYDOWN`/`WM_SYSCHAR`/`WM_SYSKEYUP`, re-register whenever recording ends); unregister on `WM_DESTROY`.
5. Run the unit and runtime verification (spec § 8), including Task 1-1/1-2 regressions.
6. Update HANDOFF.md with the actual results (spec § 7).
7. Phase end: quit the dev instance, restore settings.ini and the clipboard, `cargo build --release`, relaunch `target\release\mynotch.exe`, and smoke-test the shortcut once.
8. Commit when the session's workflow calls for it (the user's standing workflow: one plain-English commit on `main`).

## Test Expectations

New unit tests:

- `src/hotkey.rs`: `default_is_ctrl_alt_space`, `parse_accepts_any_order_and_case`, `parse_rejects_invalid_combinations`, `every_key_name_round_trips`, `recorded_presses_need_win_or_two_modifiers`.
- `src/settings.rs`: `hotkey_defaults_clears_and_falls_back`. Update `round_trips` to carry `hotkey` (both `Some` and `None`).
- `src/settings_panel.rs`: `four_rows_fit_the_content_area`, `hit_finds_each_row`.

Expected total: 58 + 8 = 66 passing.

## Verification Commands

From the repo root, in PowerShell:

```powershell
cargo build
cargo clippy --all-targets
cargo test
```

Runtime checks follow ImplementationSpec-1-3 § 8 (same setup and cleanup as Task 1-1, plus a fullscreen test window and a helper process that holds a conflicting hotkey). Phase end, after the dev instance has quit and settings.ini is restored:

```powershell
cargo build --release
Start-Process C:\Projects\ContactEstablished\MyNotch\target\release\mynotch.exe
Get-Process mynotch | Select-Object Id, Path
```

## Acceptance Criteria

- `cargo clippy --all-targets` prints no warnings; `cargo test` reports 66 passed, 0 failed.
- The Settings tab shows four rows (Color, Opacity, Position, Shortcut) inside the content area, nothing clipped; the existing rows still work.
- With the test window in the foreground, Ctrl+Alt+Space opens the notch with search focused and the pinned apps listed; Esc returns the foreground to the test window; pressing the shortcut twice opens then closes.
- Over a borderless fullscreen test window, the hidden notch reappears on the shortcut with search focused; Esc returns focus to that window and the notch hides again.
- Recording saves `hotkey=<combination>` and the new combination works while the old one no longer does. Alt combinations record without opening a system menu. Esc cancels and keeps the old shortcut working. Backspace/Delete save `hotkey=` and disable the shortcut. Bare modifiers and combinations with neither Win nor two of Ctrl/Alt/Shift (e.g. Shift+A, Ctrl+C, Alt+F4) are ignored (P1-D13). Clicking elsewhere while recording ends it and re-registers the saved shortcut.
- A combination held by another process shows "Taken by another app" on the row; MyNotch keeps running and the magnifier still works.
- The saved shortcut survives a restart.
- Task 1-1's focus checks (hover, tab and pin clicks never take focus; Esc returns focus) and Task 1-2's pinned-apps view still pass.
- HANDOFF.md describes the new state with the real verification results; settings.ini and the clipboard are restored; the test windows and helper process are closed; the rebuilt release MyNotch is running from `target\release`.

## Review Checklist

- [ ] `hotkey.rs` is pure, with no Win32 calls; `Display` order is Ctrl, Alt, Shift, Win; `parse(h.to_string()) == Some(h)` for every valid `h`.
- [ ] `hotkey=` empty → `None`; missing or unparseable → default; old settings files still load.
- [ ] Outside recording, the registered shortcut always matches `settings.hotkey` (or nothing). Every way recording ends goes through `end_typing`, which re-registers.
- [ ] `WM_HOTKEY` is ignored while a menu or the colour picker is open; it unhides before `begin_typing`; a toggle-close restores focus.
- [ ] While recording, `WM_SYSKEYDOWN`, `WM_SYSCHAR` and `WM_SYSKEYUP` never reach `DefWindowProcW`.
- [ ] `RegisterHotKey` failure only sets `hotkey_taken`; nothing panics or exits.
- [ ] `ROW_H` 43 leaves every hit area separate (unit test) and the screenshots show no clipping.
- [ ] HANDOFF.md: settings keys, file table, focus model, verification results, rough edges and test count are current; the stale `milestone-4` branch mention is corrected.
- [ ] Only the files in scope changed; `docs/` untouched; committed only if the session's workflow calls for it.
