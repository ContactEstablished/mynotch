# Task 1-1 — Typing foundation and the tab-row search bar

Status: **done 2026-10-15** in `4bfe42e`. Depends on nothing. Paired spec: [ImplementationSpec-1-1](../ImplementationSpecs/ImplementationSpec-1-1.md).

## Source Of Truth

- [Phase 1 overview](Phase-1-Overview.md): decisions P1-D2, P1-D3, P1-D4, P1-D9; gates G1, G2, G3, G7; risks.
- [roadmap](../roadmap.md), Phase 1 goals 1 and 3. [HANDOFF.md](../../../../HANDOFF.md): verification method.
- [ImplementationSpec-1-1](../ImplementationSpecs/ImplementationSpec-1-1.md): exact types, insertion points and invariants. If this task and the spec disagree, the spec wins; report the disagreement.

## Initial Starting Point

Verified against `63f6951` on 2026-10-15.

- The notch never takes focus: the window is created `WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE` (`src/app.rs:370`) and answers `WM_MOUSEACTIVATE` with `MA_NOACTIVATE` (`src/app.rs:1154`). `wndproc` (`src/app.rs:1096-1173`) handles no `WM_KEYDOWN`, `WM_CHAR`, `WM_SYSKEYDOWN` or `WM_ACTIVATE`. The message loop already calls `TranslateMessage` (`src/app.rs:394`), so `WM_CHAR` will arrive once the window has focus.
- The only focus grab is the menu's `SetForegroundWindow(hwnd)` right after a user click (`src/app.rs:1041`).
- `with_app` (`src/app.rs:991-999`) uses `try_borrow_mut`; a message that re-enters `wndproc` while `APP` is borrowed is silently dropped.
- Tab row (`src/app.rs:49-75`, `89-95`, `118-123`): six widget tabs at x = 24 + 48·t (44 × 32, y 8); the last ends at x 308. `handle_rect` is the empty stretch x 312–616, y 0–40; it shows the hovered/active tab's name and a six-dot grip, takes the ↔ cursor (`over_handle`, `src/app.rs:525-529`; `WM_SETCURSOR`, `src/app.rs:1123-1132`) and slides the notch when dragged (`src/app.rs:676-679`). The Settings tab is at x 620, the pin at x 672. `CONTENT` is Rect(24, 48, 672, 176) (`src/app.rs:69`).
- `draw_tabs` (`src/app.rs:898-941`) draws the baseline with a gap under the active tab (903-906), the tabs (909-922), the name and grip (924-933) and the pin (935-940).
- The notch's open target is `hovered || pinned || modal_open || peeking` (`src/app.rs:642`).
- `clipboard.rs` has no public text helpers: `Watcher::copy` (`src/clipboard.rs:113-128`) and the private `set_data` (`141-158`), `OpenGuard` (`161-181`) and `read_global` (`250-265`) exist; text decoding is inline in `capture` (`214-219`).
- `gfx::glyph` (`src/gfx.rs:345-367`) has no search glyph. `Canvas::text` (`src/gfx.rs:520-538`) draws one line and trims overflow with an ellipsis; `Canvas::measure` (`541-550`), `fill` (`552-555`), `line` (`558-562`), `tab` (`566-607`) and `push_clip`/`pop_clip` (`733-739`) exist.
- `src/main.rs:3-15` lists the modules; there is no `text` module.
- `Cargo.toml` line 9 already enables `Win32_UI_Input_KeyboardAndMouse` (`GetKeyState`, `SetFocus`) and `Win32_UI_WindowsAndMessaging` (`SetForegroundWindow`, `GetForegroundWindow`, `IsWindow`, `GetCaretBlinkTime`). No dependency changes are needed.
- `cargo test`: 39 passing; `cargo clippy --all-targets`: clean. A release MyNotch is running from `target\release`.

## Goal

Add a single-line text field the notch can host and the focus machinery around it, and use it for a search bar in the tab row. A magnifier button sits at the right end of the empty stretch. Clicking it takes keyboard focus (remembering the previous foreground window) and turns the stretch into an outlined search field joined to the panel. The field supports typing (including surrogate pairs), caret movement, selection, word-wise delete, select-all, cut/copy/paste through the real clipboard, and click-to-place. Esc closes and clears search and gives focus back; clicking a tab does the same and selects the tab; clicking elsewhere ends search without taking focus back. While searching, the content area shows a dim "Type to search apps" hint (Task 1-2 replaces it with results). Hovering and expanding never take focus.

## Exact Scope

Create:

- `src/text.rs` — `TextField`, `Motion`, `Command`, `command()`, `Utf16Joiner`, `draw()`, `caret_at()`, private pure helpers, unit tests.

Edit:

- `src/app.rs` — tab-row geometry (`search_rect`, `field_rect`, narrowed `handle_rect`), `Owner`, `Keyboard`, new `App` fields, `begin_typing`/`end_typing`/`open_search`, key and char handling, caret blink, drawing of the button/field/hint, `wndproc` arms for `WM_KEYDOWN`, `WM_SYSKEYDOWN`, `WM_CHAR`, `WM_ACTIVATE`, the I-beam cursor; a geometry unit test.
- `src/clipboard.rs` — `pub fn read_text(owner: HWND) -> Option<String>`, `pub fn write_text(owner: HWND, text: &str) -> bool`, sharing code with `capture` and `Watcher::copy`.
- `src/gfx.rs` — `glyph::SEARCH` (`"\u{E721}"`).
- `src/main.rs` — `mod text;`.

Nothing else.

## Non-Goals

- No search results, ranking or `Widget` trait changes (Task 1-2). Enter in the field does nothing yet.
- No global shortcut, `RegisterHotKey`, settings key or Settings row (Task 1-3).
- No text fields inside widgets (Phase 4, per P1-D12).
- No multi-line editing, undo/redo, mouse drag-selection, double-click word selection, IME composition UI, or grapheme-cluster handling (char boundaries only).
- No change to `WM_MOUSEACTIVATE` (stays `MA_NOACTIVATE`) or to the window's extended styles.
- No dependency or `Cargo.toml` changes.
- No change to Alt+F4 behaviour when the notch is *not* holding the keyboard (pre-existing).
- Do not revert or commit unrelated changes. `docs/` is untracked at kickoff (the roadmap and these Phase 1 docs): don't delete, move or rewrite it, and don't fold it into this task's code commit unless the user asks.

## Dependencies

None. Tasks 1-2 and 1-3 build on the names defined here (`text::TextField`, `Owner`, `Keyboard`, `begin_typing`, `end_typing`, `open_search`, `search_rect`, `field_rect`).

## Step-by-step Work

1. Write `src/text.rs` as specified: the editing model first, with its unit tests, then `command`, `Utf16Joiner`, the pure layout helpers and their tests, then `draw` and `caret_at`. Add `mod text;` to `src/main.rs`.
2. Add `glyph::SEARCH` to `src/gfx.rs`.
3. Add `read_text`/`write_text` to `src/clipboard.rs`, factoring the UTF-16 decode out of `capture` and the text arm out of `Watcher::copy` so behaviour there is unchanged.
4. In `src/app.rs`: add `search_rect` and `field_rect`, narrow `handle_rect`, and add the geometry test.
5. Add `Owner`, `Keyboard`, the `App` fields, `begin_typing`, `end_typing` and `open_search`, following the focus invariants in the spec (state first, then focus calls).
6. Route input: `on_press` (tab click closes search, field click places the caret, magnifier opens search, no slide-drag or widget clicks while searching); `wndproc` arms for keys, chars and activation; I-beam over the field.
7. Keep the notch open while the keyboard is held (`src/app.rs:642`); blink the caret in `tick`; add the foreground safety net.
8. Draw the magnifier button, the open field (with the baseline gap under it and no active widget tab), and the content hint.
9. Run the verification below. If the session ends here, relaunch the release exe.

## Test Expectations

New unit tests in `src/text.rs` (names may be adjusted, coverage may not):

- `insert_replaces_the_selection_and_sanitises_control_chars`
- `insert_respects_max_chars`
- `backspace_and_delete_by_char_and_by_word`
- `caret_motions_extend_and_collapse_the_selection`
- `word_motions_skip_spaces_and_punctuation`
- `indices_stay_on_char_boundaries_with_multibyte_text`
- `cut_and_selected_return_the_selection`
- `set_caret_clamps_and_rounds_down_to_a_char_boundary`
- `command_maps_editing_keys`
- `joiner_pairs_surrogates_and_drops_controls_and_orphans`
- `scroll_offset_keeps_the_caret_visible`
- `nearest_boundary_picks_the_closest_char_edge`

In `src/clipboard.rs`: `utf16_text_stops_at_the_first_nul`. In `src/app.rs`: `search_button_and_field_share_the_tab_row_stretch`.

Expected total: 39 + 14 = 53 passing.

## Verification Commands

From the repo root (`C:\Projects\ContactEstablished\MyNotch`), in PowerShell:

```powershell
cargo build
cargo clippy --all-targets
cargo test
```

Runtime checks (G1) follow ImplementationSpec-1-1 § Verification. They need the running release instance quit first and settings.ini backed up:

```powershell
$scratch = Join-Path $env:TEMP 'mynotch-phase1'   # or your session scratchpad
New-Item -ItemType Directory -Force $scratch | Out-Null
Copy-Item "$env:APPDATA\MyNotch\settings.ini" "$scratch\settings.ini.bak"
Add-Type -Namespace Notch -Name Win -MemberDefinition @'
[DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern IntPtr FindWindow(string cls, string title);
[DllImport("user32.dll")] public static extern bool PostMessage(IntPtr hwnd, uint msg, IntPtr w, IntPtr l);
'@
$h = [Notch.Win]::FindWindow('MyNotchWindow', $null)
if ($h -ne [IntPtr]::Zero) { [void][Notch.Win]::PostMessage($h, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero) }
Wait-Process -Name mynotch -Timeout 10 -ErrorAction SilentlyContinue
Start-Process C:\Projects\ContactEstablished\MyNotch\target\debug\mynotch.exe
```

Afterwards, quit the dev instance the same way, then:

```powershell
Copy-Item "$scratch\settings.ini.bak" "$env:APPDATA\MyNotch\settings.ini" -Force
Start-Process C:\Projects\ContactEstablished\MyNotch\target\release\mynotch.exe
```

## Acceptance Criteria

- `cargo build` succeeds; `cargo clippy --all-targets` prints no warnings; `cargo test` reports 53 passed, 0 failed.
- With a test window in the foreground, hovering the notch until it opens, and clicking a widget tab or the pin, leave the test window in the foreground.
- Clicking the magnifier makes the notch the foreground window; the field shows an outlined tab joined to the panel, no widget tab is drawn active, and the content area shows "Type to search apps".
- Typed text appears in the field, including "é" and "😀" sent as Unicode input; nothing typed reaches the test window.
- Left/Right, Ctrl+Left/Right, Home/End (with and without Shift), Backspace, Delete, Ctrl+Backspace, Ctrl+Delete, Ctrl+A, Ctrl+C, Ctrl+X and Ctrl+V behave as specified; text copied from the field is on the system clipboard and in MyNotch's Clipboard tab; pasted multi-line text arrives as one line.
- Clicking inside the field moves the caret; Shift+click extends the selection; the ↔ cursor and slide-drag don't apply over the magnifier or while searching; the I-beam shows over the field.
- Esc closes and clears search and the test window is foreground again. Clicking a tab while searching does the same and selects that tab. Clicking the test window while searching ends search and the notch does not take focus back. Alt+F4 while searching closes search and MyNotch keeps running.
- The notch stays open while searching even with the cursor away, and collapses after search ends.
- settings.ini is restored, the test window is closed, the cursor is back where it started, and a release MyNotch is running when the session ends.

## Review Checklist

- [ ] `text.rs` has no Win32 calls; `draw` and `caret_at` only use `Canvas`. Pure helpers carry the logic that tests cover.
- [ ] Every `TextField` mutation leaves `caret` and `anchor` on char boundaries and within `text.len()`; `text` never contains control chars or more than `max_chars` chars.
- [ ] `end_typing` clears `keyboard` and `search` before calling `SetForegroundWindow`; no code relies on a re-entrant `WM_ACTIVATE` being handled.
- [ ] `begin_typing` is reached only from a click (magnifier) and backs out if the notch did not become foreground; hover/expand paths never call `SetForegroundWindow` or `SetFocus`.
- [ ] `WM_ACTIVATE` still falls through to `DefWindowProcW`; `WM_MOUSEACTIVATE` still returns `MA_NOACTIVATE`.
- [ ] Key and char messages are passed to `DefWindowProcW` (or ignored) when the keyboard isn't held.
- [ ] Ctrl+Alt (AltGr) chords are not editing commands.
- [ ] `capture` and `Watcher::copy` behave exactly as before.
- [ ] `handle_rect` no longer includes the magnifier; `on_press` never starts a slide while searching.
- [ ] Runtime evidence covers every acceptance bullet, with screenshots of the closed bar, the open field (empty and with a selection) and the hint.
- [ ] Only the five files in scope changed; `docs/` untouched; committed only if the session's workflow calls for it.
