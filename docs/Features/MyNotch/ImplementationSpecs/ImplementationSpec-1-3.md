# Implementation spec 1-3 — Global shortcut, Settings recorder, phase end

Companion: [Task 1-3](../Tasks/Task-1-3.md). Overview: [Phase 1](../Tasks/Phase-1-Overview.md). Builds on [ImplementationSpec-1-1](ImplementationSpec-1-1.md) and [ImplementationSpec-1-2](ImplementationSpec-1-2.md). Line numbers in `settings.rs`, `settings_panel.rs`, `desktop.rs` and `main.rs` are from `63f6951`; Tasks 1-1/1-2 don't touch those files, except `main.rs`, which gains `mod text;`. `src/app.rs` numbers are baseline; find code by function name.

## 1. Files and boundaries

| File | Change |
| --- | --- |
| `src/hotkey.rs` (new) | `Hotkey`, key names, validation. Pure: no `windows` import. |
| `src/main.rs` | `mod hotkey;` after `mod gfx;` (alphabetical). |
| `src/settings.rs` | `hotkey: Option<Hotkey>`. |
| `src/settings_panel.rs` | The fourth row. |
| `src/app.rs` | Registration, `WM_HOTKEY`, the recorder. |
| `HANDOFF.md` | Phase-end state (§ 7). |

## 2. `src/hotkey.rs`

Module doc: a global shortcut — modifiers plus one key — read from and shown as text like "Ctrl+Alt+Space". Pure, so it is unit-tested; `app.rs` registers it.

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hotkey {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
    /// Virtual-key code of the non-modifier key; always one in the key table.
    pub vk: u16,
}

impl Hotkey {
    /// Ctrl+Alt+Space.
    pub const DEFAULT: Hotkey = Hotkey { ctrl: true, alt: true, shift: false, win: false, vk: 0x20 };

    /// From a key pressed while recording. `None` unless the result is valid (see below).
    pub fn new(ctrl: bool, alt: bool, shift: bool, win: bool, vk: u16) -> Option<Hotkey>

    /// "Ctrl+Alt+Space": '+'-separated, any order and case, spaces around parts allowed;
    /// "Control" and "Windows" are accepted as well. `None` unless valid.
    pub fn parse(s: &str) -> Option<Hotkey>

    /// RegisterHotKey's modifier bits: MOD_ALT 1, MOD_CONTROL 2, MOD_SHIFT 4, MOD_WIN 8
    /// (without MOD_NOREPEAT, which app.rs adds).
    pub fn modifiers(&self) -> u32
}

/// "Ctrl+Alt+Shift+Win+Key": modifiers in that fixed order, then the key's name.
impl std::fmt::Display for Hotkey

/// Whether `vk` is a modifier key on its own: Shift, Ctrl, Alt (0x10–0x12), the Windows
/// keys (0x5B, 0x5C) and their left/right forms (0xA0–0xA5).
pub fn is_modifier(vk: u16) -> bool
```

**Valid** means all of:

- `win`, or at least two of `ctrl`, `alt`, `shift` (P1-D13);
- `vk` is in the key table.

The two-modifier rule keeps everyday app shortcuts (Ctrl+C, Ctrl+S, Alt+F, Alt+F4) out of reach: a registered hotkey is delivered to MyNotch instead of the focused app, in every app.

**Key table** (`vk` → name, one static table used both ways, names matched case-insensitively):

| VK | Names |
| --- | --- |
| 0x41–0x5A | `A` … `Z` |
| 0x30–0x39 | `0` … `9` |
| 0x70–0x87 | `F1` … `F24` |
| 0x20 | `Space` |
| 0x25, 0x26, 0x27, 0x28 | `Left`, `Up`, `Right`, `Down` |
| 0x24, 0x23, 0x21, 0x22, 0x2D, 0x2E | `Home`, `End`, `PageUp`, `PageDown`, `Insert`, `Delete` |
| 0xBA, 0xBB, 0xBC, 0xBD, 0xBE, 0xBF, 0xC0, 0xDB, 0xDC, 0xDD, 0xDE | `;`, `=`, `,`, `-`, `.`, `/`, `` ` ``, `[`, `\`, `]`, `'` |

Punctuation names are the US-layout labels of the OEM keys; on other layouts the label may differ but the saved key code is what registers. No name contains `+`, so splitting on `+` is unambiguous. Tab, Esc, Enter and Backspace are deliberately absent: Esc and Backspace control the recorder, and Tab/Enter combinations are too easy to hit by accident.

**`parse`**: split on `+`, trim each part; an empty part → `None`. Every part but the last must be a distinct modifier name; the last must be a key name. Then apply the validity rules.

**Tests** (`#[cfg(test)] mod tests`):

- `default_is_ctrl_alt_space` — `DEFAULT.to_string() == "Ctrl+Alt+Space"`, `parse("Ctrl+Alt+Space") == Some(DEFAULT)`, `DEFAULT.modifiers() == 3`.
- `parse_accepts_any_order_and_case` — `parse("alt + control + space") == Some(DEFAULT)`; `parse("win+shift+f12")` displays as `"Shift+Win+F12"`; `parse("Ctrl+Alt+pageup")` displays as `"Ctrl+Alt+PageUp"`.
- `parse_rejects_invalid_combinations` — `""`, `"Space"`, `"Shift+A"`, `"Ctrl+C"`, `"Alt+F4"`, `"Ctrl+Alt"`, `"Ctrl+Ctrl+A"`, `"Ctrl+Alt+A+B"`, `"Ctrl+Alt+Banana"`, `"Ctrl+Alt+"` all → `None`; `"Ctrl+Alt+F4"`, `"Ctrl+Shift+K"`, `"Alt+Shift+Q"` and `"Win+N"` → `Some`.
- `every_key_name_round_trips` — for every table entry, `parse(&format!("Ctrl+Alt+{name}"))` gives that `vk`, and `Display` reproduces the string.
- `recorded_presses_need_win_or_two_modifiers` — `new(false, false, true, false, 0x41)` → `None` (Shift+A); `new(true, false, false, false, 0x41)` → `None` (Ctrl+A); `new(true, false, true, false, 0x41)` → `Some` (Ctrl+Shift+A); `new(false, false, false, true, 0x41)` → `Some` (Win+A); `new(true, true, false, false, 0x11)` → `None` (a modifier is not a key); `new(true, true, false, false, 0x09)` → `None` (Tab isn't in the table); `is_modifier` is true for 0x10, 0x11, 0x12, 0x5B, 0x5C, 0xA0–0xA5 and false for 0x41.

## 3. `src/settings.rs`

- `use crate::hotkey::Hotkey;`
- Field, after `position` (`src/settings.rs:28-29`):

  ```rust
  /// The global shortcut that opens app search; `None` = no shortcut.
  pub hotkey: Option<Hotkey>,
  ```

- `Default` (32-45): `hotkey: Some(Hotkey::DEFAULT)`. A settings file from before Phase 1 has no `hotkey` line, so it gets the default.
- `parse`, new arm in the key `match` (75-106): an empty value → `None`; else `Some(Hotkey::parse(value).unwrap_or(Hotkey::DEFAULT))`. An unparseable value falls back to the default rather than silently disabling the shortcut.
- `serialize` (111-126): append `hotkey={}\n` with the `Display` text, or an empty value for `None`. Always write the line, so "none" survives a round trip as distinct from "never set".
- Tests: in `round_trips` (138-155) add `hotkey: Some(Hotkey { ctrl: true, alt: false, shift: true, win: false, vk: 0x4B })` to the literal and add a `None` round trip. New `hotkey_defaults_clears_and_falls_back`: `parse("")` → `Some(DEFAULT)`; `parse("hotkey=")` → `None`; `parse("hotkey=Ctrl+Shift+K")` → that key; `parse("hotkey=Shift+K")` and `parse("hotkey=nonsense")` → `Some(DEFAULT)`.

## 4. `src/settings_panel.rs`

- `ROW_H` (`src/settings_panel.rs:41`): `56.0` → `43.0`, with the comment "Four rows fill the 176-px content area: 4 + 4 × 43." Row centres become 25.5, 68.5, 111.5, 154.5. Everything else is already positioned from `row_centre` (66-68).
- `Hit` (52-60): add `/// The Shortcut chip: start recording a new shortcut.` `Shortcut`.
- Constants: `SHORTCUT_W: f32 = 168.0` (room for "Ctrl+Alt+Shift+PageDown" in `Font::Small`); `WARN: Rgb = Rgb(255, 176, 92)` (amber, for the conflict note).
- `fn shortcut_chip() -> Rect` — `Rect::new(LABEL_W, row_centre(3) - CHIP.1 / 2.0, SHORTCUT_W, CHIP.1)`, beside `center_chip` (82-84).
- `hit` (99-107): before the Center check, `if shortcut_chip().contains(x, y) { return Some(Hit::Shortcut); }`.
- New public type:

  ```rust
  /// What the Shortcut row shows besides the saved shortcut.
  #[derive(Clone, Copy, Default)]
  pub struct ShortcutRow {
      /// Waiting for the user to press a combination.
      pub recording: bool,
      /// Windows refused to register the saved shortcut.
      pub taken: bool,
  }
  ```

- `draw` (109-158) gains a `shortcut: ShortcutRow` parameter. After the Position row, `label(3, "Shortcut")` and the chip, filled as the Center chip is (`CHIP.1 / 2.0` radius):

  | State | Chip fill | Chip text | Note after the chip (`after(chip)`) |
  | --- | --- | --- | --- |
  | recording | `Ink::Hover` | "Press a shortcut…" in `Ink::Accent` | "Esc cancels · Backspace clears" in `Ink::Dim` |
  | saved, taken | `Track`, or `Hover` while hovered | the shortcut in `Ink::Text` | "Taken by another app" in `Ink::Rgb(WARN)` |
  | saved, registered | `Track`, or `Hover` while hovered | the shortcut in `Ink::Text` | "Opens app search from anywhere" in `Ink::Dim` |
  | none | `Track`, or `Hover` while hovered | "None" in `Ink::Dim` | "Click to set a shortcut" in `Ink::Dim` |

  Chip text is `Font::Small`, centred; notes are `Font::Small`.
- Tests (extend the imports at 253):
  - `four_rows_fit_the_content_area` — `ROW_Y + ROW_H * 4.0 <= 176.0` (176 is `CONTENT.h` in `app.rs`), and `shortcut_chip().y + shortcut_chip().h <= 176.0`.
  - `hit_finds_each_row` — the centres of `swatch(0)`, `track()`, `center_chip()` and `shortcut_chip()` hit `Preset`, `Opacity`, `Center` and `Shortcut` (use `matches!`). `hit(LABEL_W + 10.0, 133.0)`, between the Position and Shortcut chips, is `None`.

## 5. `src/app.rs`

### 5.1 Additions

- Imports: `RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_NOREPEAT, VK_BACK, VK_DELETE, VK_ESCAPE, VK_LWIN, VK_RWIN` from `KeyboardAndMouse`; `use crate::hotkey::{self, Hotkey};`; `settings_panel::ShortcutRow`. `WM_HOTKEY`, `WM_SYSCHAR` and `WM_SYSKEYUP` come from `WindowsAndMessaging::*`.
- `const HOTKEY_ID: i32 = 1;` beside `TRAY_ID` (`src/app.rs:43` at baseline).
- `Owner` gains `/// The Settings tab's shortcut recorder.` `Shortcut`.
- `App` gains `/// Windows refused to register the saved shortcut.` `hotkey_taken: bool`, initialised `false`.

### 5.2 Registration

```rust
/// Registers `settings.hotkey` (replacing any earlier registration), noting a refusal.
fn register_hotkey(&mut self)
```

`UnregisterHotKey(Some(self.hwnd), HOTKEY_ID)` (ignore the result), then `hotkey_taken = false`; if `settings.hotkey` is `Some(h)`, `hotkey_taken = RegisterHotKey(Some(self.hwnd), HOTKEY_ID, HOT_KEY_MODIFIERS(h.modifiers()) | MOD_NOREPEAT, u32::from(h.vk)).is_err()`. Set `dirty`. `MOD_NOREPEAT` stops a held shortcut from toggling repeatedly.

Call sites: `run`, right after `App::new` (baseline `src/app.rs:384`), before the app moves into `APP`; and `end_typing`'s `Owner::Shortcut` cleanup (§ 5.4). Unregister in `WM_DESTROY` (baseline 1162-1166) before `PostQuitMessage`.

Invariant H1: outside recording, the registered shortcut is exactly `settings.hotkey` (nothing when `None` or taken). During recording nothing is registered, so the keys arrive as ordinary key messages.

### 5.3 `WM_HOTKEY`

Add the arm `WM_HOTKEY if wparam.0 == HOTKEY_ID as usize => { with_app(|a| a.on_hotkey()); LRESULT(0) }`.

```rust
/// The global shortcut: toggles search from any app, even while hidden over a fullscreen one.
fn on_hotkey(&mut self)
```

- `modal_open` (the menu or colour picker is up) → ignore.
- Typing with `Owner::Search` → `end_typing(true)` (P1-D11: same as Esc).
- Typing with `Owner::Shortcut` → ignore. This can't happen, since nothing is registered while recording (H1).
- Not typing → `self.unhide()`, then `self.open_search()`.

Windows normally lets the process that receives a hotkey take the foreground: the hotkey is its last input event. `begin_typing` relies on that. Its own `GetForegroundWindow` check (Spec 1-1 § 6.5) catches a refusal, and runtime checks T2 and T5 must show the grab works, including over a fullscreen window. If it doesn't, stop and report (Overview, foreground-lock risk) rather than adding workarounds.

```rust
/// Brings the notch back from fullscreen hiding.
fn unhide(&mut self)
```

If `hidden`: `hidden = false` and `ShowWindow(self.hwnd, SW_SHOWNOACTIVATE)`. The window may be `SW_HIDE`'d at fade 0 (`tick`, baseline 599-605), and a hidden window can't take the foreground. Always set `next_fullscreen_check = Instant::now() + FULLSCREEN_POLL`, so the poll (583-594) doesn't run between the shortcut and the notch taking the foreground. Once the notch is foreground, `foreground_is_fullscreen` returns false because it ignores our own process (`src/desktop.rs:84`), so the notch stays visible while searching. After Esc gives focus back to the fullscreen app, the next poll hides it again (P1-D10). `fade` climbs back to 1 by the usual `tick` path.

### 5.4 The recorder

```rust
/// Starts recording a new shortcut (the Shortcut chip was clicked).
fn start_recording(&mut self)
```

`UnregisterHotKey(Some(self.hwnd), HOTKEY_ID)`, so the current combination arrives as `WM_KEYDOWN`/`WM_SYSKEYDOWN` and can be re-recorded. Then `begin_typing(Owner::Shortcut)`; if that returns `false`, call `register_hotkey()` again (H1). Set `dirty`.

```rust
/// A key pressed while recording.
fn record_key(&mut self, vk: u16)
```

Read the modifiers with `GetKeyState`, which is in step with the key message being handled: `ctrl` = `VK_CONTROL`, `alt` = `VK_MENU`, `shift` = `VK_SHIFT`, `win` = `VK_LWIN || VK_RWIN`. Let *bare* mean none of the four is held. Then, in order:

1. `hotkey::is_modifier(vk)` → return (wait for the real key).
2. Bare Esc → `end_typing(true)`. Cancel: the old shortcut is re-registered by the cleanup.
3. Bare Backspace or bare Delete → `settings.hotkey = None`, `settings.save()`, `end_typing(true)`.
4. `Hotkey::new(ctrl, alt, shift, win, vk)` is `Some(h)` → `settings.hotkey = Some(h)`, `settings.save()`, `end_typing(true)`.
5. Otherwise (neither Win nor two of Ctrl/Alt/Shift, e.g. Ctrl+C or Alt+F4; or a key outside the table) → ignore and keep recording.

Settings change before `end_typing`, so its cleanup registers the new value. This is Task 1-1's state-first rule (F4) applied to the hotkey.

`end_typing`'s owner cleanup gains `Owner::Shortcut => self.register_hotkey()`. It runs on every exit: Esc, a recorded key, clearing, a tab click, a press elsewhere in the notch, `WA_INACTIVE` (a click outside, the Start menu, Alt+Tab), and the `tick` safety net.

### 5.5 Routing

- **`on_key`**: when the owner is `Shortcut`, call `record_key(vk)` for both `WM_KEYDOWN` and `WM_SYSKEYDOWN` and return `true`. Task 1-1's Alt+F4 rule applies only to `Owner::Search`; while recording, Alt+F4 reaches `record_key`, which refuses it (step 5), so it neither quits MyNotch nor gets saved.
- **`on_char`**: unchanged. While typing it swallows chars, and the `Shortcut` owner inserts nothing.
- **`wndproc`**: add `WM_SYSCHAR | WM_SYSKEYUP`: if the owner is `Shortcut`, return `LRESULT(0)`; else `DefWindowProcW`. This stops Alt+Space opening the system menu and an Alt tap entering menu mode while recording. (Expose a small `fn recording(&self) -> bool` for this and for drawing.)
- **`on_press`** while recording, after the PIN branch (which works as usual): a tab hit → Task 1-1's tab branch (`end_typing(true)`, then `select_tab`). Any other press, including on the chip itself, → `end_typing(true)` and nothing else. Put this check before the search and Settings branches.
- **`on_press`** Settings branch (baseline 680-697): `Hit::Shortcut => self.start_recording()`.

### 5.6 Drawing

At the `settings_panel::draw` call (baseline `src/app.rs:853`), pass `ShortcutRow { recording: self.recording(), taken: self.hotkey_taken }`. Recording keeps the notch open through Task 1-1's `keyboard.is_some()` term in the open target.

## 6. Edge cases

- **The shortcut is already taken at startup** (e.g. by a launcher): `hotkey_taken` is set; the row shows the conflict; the magnifier still works; nothing else changes.
- **Recording a combination another app owns**: it's saved (the chip shows it) and the row shows "Taken by another app". Re-recording or clearing fixes it. It's re-tried at every start.
- **Windows-reserved combinations** (Win+L, many Win+letters, F12): either the shell acts before MyNotch sees them while recording (nothing is recorded; a Start/shell action may deactivate the notch and end recording via `WA_INACTIVE`), or `RegisterHotKey` fails (shown as taken).
- **Common app shortcuts.** The validity rule (P1-D13: Win, or two of Ctrl/Alt/Shift) refuses single-modifier shortcuts like Ctrl+C or Alt+F4. Two-modifier app shortcuts (e.g. Ctrl+Shift+T in browsers) can still be recorded; that is the user's explicit choice, and re-recording or clearing undoes it.
- **AltGr while recording** reads as Ctrl+Alt+key and records as such. That's fine for a shortcut.
- **The shortcut during the menu or colour picker**: ignored (`modal_open`).
- **The shortcut while dragging**: opens search; the drag still ends when the button comes up (`tick`, baseline 574-580).
- **A fullscreen app with exclusive display mode** may minimise when the notch takes the foreground. Accepted; borderless-fullscreen apps behave as in P1-D10.
- **The previous window is the fullscreen app and it closed meanwhile**: `IsWindow` fails; nothing is restored.

## 7. `HANDOFF.md` (phase-end update)

Edit in place, keeping its structure and tone. Record only results you actually observed.

- Heading and "Current state": state after Phase 1, on `main` (fix the stale "branch `milestone-4`" wording at `HANDOFF.md:27`), the test count (66) and clippy status.
- "Decisions made": add a Phase 1 line pointing to `docs/Features/MyNotch/roadmap.md` and `Tasks/Phase-1-Overview.md` (search bar in the tab row; Ctrl+Alt+Space, recordable; focus model).
- "What it does": the magnifier and search bar (empty query = pinned apps; ranking; keys; Enter/click launch); the global shortcut (toggle; works over fullscreen); the Settings tab's fourth row; the tab row's description (the handle now stops at the magnifier).
- Settings keys: add `hotkey` (e.g. `Ctrl+Alt+Space`; empty = none).
- "Verified": a Phase 1 entry summarising the runtime checks run (focus, typing/editing, results/launch, shortcut, fullscreen, recorder, conflict) and anything not verified (for example, a non-US layout or a real IME).
- File table: `src/text.rs` and `src/hotkey.rs`; update the rows for `app.rs` (focus/typing, search routing, hotkey), `widgets/mod.rs` (search hooks), `widgets/apps.rs` (search results), `clipboard.rs` (`read_text`/`write_text`) and `settings_panel.rs` (Shortcut row).
- "How it works": the focus model. The window still never activates on a click (`WS_EX_NOACTIVATE`, `MA_NOACTIVATE`); it takes the foreground explicitly for typing, remembers the previous window, gives it back on Esc/toggle/tab click, and lets go on `WA_INACTIVE` or a launch.
- "Known rough edges": remove "no search in the picker" (keep "the app list loads once per launch"). Add: no undo, drag-selection or double-click selection in fields; IME composition window not positioned; shortcut key names are US-layout labels; some Win+ combinations can't be recorded; two-modifier combinations that apps rely on (e.g. Ctrl+Shift+T) can still be recorded; the slide handle is 48 px narrower (grip centre x 440), which the regression-script coordinates must reflect.
- "Next features": note that the text field exists; widget-hosted fields come with Phase 4.

## 8. Verification

### 8.1 Build and unit tests

```powershell
cargo build
cargo clippy --all-targets
cargo test
```

Expect no warnings and 66 passing tests.

### 8.2 Runtime (G1)

Setup as in ImplementationSpec-1-1 § 8.2 (back up settings.ini and the clipboard, quit the release instance, run `target\debug\mynotch.exe`, start the "MyNotch focus test" form, record the cursor position, and record the PIDs of any app you will launch). Also:

- **Fullscreen form**: a second WinForms process titled "MyNotch fullscreen test", with `FormBorderStyle = 'None'`, `StartPosition = 'Manual'` and `Bounds = [Windows.Forms.Screen]::PrimaryScreen.Bounds`. It is not maximised, so `covers_monitor` counts it as fullscreen (`src/desktop.rs:97-107`). Start it only for T5, and close it after.
- **Conflict holder**: a hidden PowerShell process that registers Ctrl+Alt+F9 for its own thread and sleeps:

```powershell
$holder = Start-Process pwsh -PassThru -WindowStyle Hidden -ArgumentList '-NoProfile', '-Command', @'
Add-Type -Namespace Hk -Name Win -MemberDefinition '[DllImport("user32.dll")] public static extern bool RegisterHotKey(IntPtr h, int id, uint mods, uint vk);'
if (-not [Hk.Win]::RegisterHotKey([IntPtr]::Zero, 1, 0x4003, 0x78)) { exit 1 }   # Ctrl+Alt+F9, MOD_NOREPEAT
Start-Sleep -Seconds 900
'@
```

Shortcut chords are sent with `SendInput` as real key events (e.g. Ctrl down, Alt down, Space down/up, Alt up, Ctrl up). `RegisterHotKey` sees injected input. The Settings tab is at logical (642, 24). The Shortcut chip's centre is at content (LABEL_W + 84, 154.5), i.e. logical (24 + 96 + 84, 48 + 154.5) = (204, 202.5).

| # | Steps | Expect |
| --- | --- | --- |
| T1 | Open the Settings tab; screenshot. | Four rows inside the panel; the chip reads "Ctrl+Alt+Space"; the note reads "Opens app search from anywhere", or "Taken by another app" (record which; if taken, record a free combination first and use it for T2–T4). Swatch rings, slider and chips unclipped. |
| T2 | Click the test form. Send the shortcut. | Within 300 ms the notch is foreground, expanded, with the search field and the pinned apps. |
| T3 | Type `calc`; Esc. | Calculator highlighted; after Esc the test form is foreground and the notch collapses once the cursor is away. |
| T4 | Shortcut, shortcut. Then shortcut, `calc`, Enter. | Open then closed, with the test form foreground again. Then Calculator launches and is foreground; close only the new Calculator. |
| T5 | Start the fullscreen form; click it; wait 1.5 s. Shortcut; screenshot. Esc; wait 1.5 s. | The notch hides (`IsWindowVisible` false). On the shortcut it reappears with search focused (notch foreground). After Esc the fullscreen form is foreground and the notch is hidden again. Close the fullscreen form. |
| T6 | Settings tab; click the chip; screenshot. Send Ctrl+Alt+K. | "Press a shortcut…" while recording, with the notch foreground. Then the chip reads "Ctrl+Alt+K"; settings.ini has `hotkey=Ctrl+Alt+K`; the test form is foreground. Ctrl+Alt+Space now does nothing; Ctrl+Alt+K opens search (Esc). |
| T7 | Click the chip; send Alt+Shift+Q; screenshot. | Saved as "Alt+Shift+Q"; no system menu appeared; it opens search (Esc). |
| T8 | Click the chip; Esc. | The chip still reads "Alt+Shift+Q" and it still works. |
| T9 | Click the chip; send Shift+A; press and release Ctrl alone; send Alt+F4. | Still recording after each, and MyNotch still running. Then Esc. |
| T10 | Click the chip; Backspace. | The chip reads "None"; settings.ini has `hotkey=` (empty); neither Alt+Shift+Q nor Ctrl+Alt+Space opens search; the magnifier still does. |
| T11 | Start the holder. Click the chip; send Ctrl+Alt+F9. Use the magnifier once. Stop the holder; record Ctrl+Alt+Space. | The chip reads "Ctrl+Alt+F9" with "Taken by another app"; MyNotch runs; the magnifier search works. Afterwards the note is gone and Ctrl+Alt+Space works. |
| T12 | Click the chip; click the test form. Then send Ctrl+Alt+Space. | Recording ends (the chip shows "Ctrl+Alt+Space" again on hover); the shortcut opens search (Esc). |
| T13 | Record Ctrl+Alt+K; quit the dev MyNotch; start it again; open Settings. Send Ctrl+Alt+K. | The chip reads "Ctrl+Alt+K" after restart and the shortcut works. (The settings restore at cleanup puts the user's file back.) |
| T14 | Regressions: Task 1-1 R1 (hover, tab and pin never take focus) and R14 (Esc returns focus); Task 1-2 S1 (search shows the pins). Click the opacity slider's middle, then the Center chip. | All as before; opacity and Center still work with the 43 px rows. |

**Cleanup, then phase end.**

1. Stop the holder and close the test forms. Quit the dev MyNotch. Restore settings.ini and the clipboard. Restore the cursor. Confirm no test-launched app is left running.
2. Update HANDOFF.md (§ 7) with the results above.
3. `cargo build --release`; `Start-Process C:\Projects\ContactEstablished\MyNotch\target\release\mynotch.exe`; `Get-Process mynotch` shows one process from `target\release`.
4. Smoke test on the relaunched release: press Ctrl+Alt+Space from the current foreground window, confirm search opens, Esc, and confirm focus went back. If the combination is taken on this machine, say so in the report.
5. Commit when the session's workflow calls for it (the user's standing workflow: a plain-English commit straight to `main`, G7). Leave `docs/` as the user/coordinator decides.

Report every check's actual result with screenshot paths. Don't claim a check you didn't run.
