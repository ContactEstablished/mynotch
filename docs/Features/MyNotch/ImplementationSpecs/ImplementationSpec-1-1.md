# Implementation spec 1-1 — Text field, focus and the tab-row search bar

Companion: [Task 1-1](../Tasks/Task-1-1.md). Overview: [Phase 1](../Tasks/Phase-1-Overview.md). Baseline `63f6951`; every line number below was read on 2026-10-15. Types and signatures here are contracts that Tasks 1-2 and 1-3 build on; keep the names.

## 1. Files and boundaries

| File | Change |
| --- | --- |
| `src/text.rs` (new) | Editing model, key mapping, UTF-16 joiner, drawing. Pure apart from `draw`/`caret_at`, which only touch `Canvas`. |
| `src/main.rs` | `mod text;` between `mod stats;` (line 13) and `mod timer;` (line 14). |
| `src/gfx.rs` | `glyph::SEARCH`. |
| `src/clipboard.rs` | `read_text`, `write_text`, and two private helpers shared with existing code. |
| `src/app.rs` | Geometry, focus, input routing, drawing. |

`text.rs` must not import `windows`. Virtual-key codes are plain `u16` constants inside it so the mapping is testable without Win32 types.

## 2. `src/text.rs`

Module doc: a single-line text field — the editing model, key mapping, a UTF-16 joiner for `WM_CHAR`, and drawing. Indices are byte offsets on char boundaries; grapheme clusters are out of scope.

### 2.1 Types

```rust
pub struct TextField {
    text: String,
    /// Where typing goes; the moving end of the selection.
    caret: usize,
    /// The fixed end of the selection; equal to `caret` when nothing is selected.
    anchor: usize,
    max_chars: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion { Left, Right, WordLeft, WordRight, Home, End }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    /// Move the caret; `true` extends the selection (Shift held).
    Move(Motion, bool),
    /// `true` = by word (Ctrl held).
    Backspace(bool),
    Delete(bool),
    SelectAll,
    Copy,
    Cut,
    Paste,
    Submit,
    Cancel,
}

/// Turns `WM_CHAR` UTF-16 code units into chars.
#[derive(Default)]
pub struct Utf16Joiner { high: Option<u16> }
```

### 2.2 Invariants (every public method preserves them)

- I1. `caret <= text.len()` and `anchor <= text.len()`, and both satisfy `text.is_char_boundary`.
- I2. `text.chars().count() <= max_chars`.
- I3. `text` contains no `char::is_control` characters.
- I4. `anchor == caret` ⇔ no selection; `selection()` is `min..max` of the two.

### 2.3 Methods

| Signature | Behaviour |
| --- | --- |
| `pub fn new(max_chars: usize) -> Self` | Empty; caret = anchor = 0. |
| `pub fn text(&self) -> &str`, `pub fn is_empty(&self) -> bool` | Accessors. |
| `pub fn clear(&mut self)` | Empty text; caret = anchor = 0. |
| `pub fn selection(&self) -> Range<usize>` | `min(anchor, caret)..max(anchor, caret)`. |
| `pub fn selected(&self) -> &str` | `&text[selection()]`. |
| `pub fn insert(&mut self, s: &str)` | Sanitise `s`: `"\r\n"` → one space; any remaining `'\r'`, `'\n'`, `'\t'` → space; drop every other `is_control` char. Budget = `max_chars − chars(text) + chars(selected)`; keep only the first `budget` chars. If what's left is empty, do nothing (the selection stays). Otherwise replace the selection with it; caret = anchor = end of the inserted text. |
| `pub fn backspace(&mut self, word: bool)` | Selection → delete it. Else at 0 → nothing. Else delete `word_left(caret)..caret` if `word`, else the previous char. |
| `pub fn delete(&mut self, word: bool)` | Selection → delete it. Else at end → nothing. Else delete `caret..word_right(caret)` if `word`, else the next char. |
| `pub fn move_caret(&mut self, m: Motion, extend: bool)` | If `!extend` and there is a selection and `m` is `Left`/`Right`: collapse to the selection's start/end and stop. Otherwise compute the target from `caret` (`Left`/`Right` one char; `WordLeft`/`WordRight` via the helpers; `Home` 0; `End` len), set `caret`, and set `anchor = caret` unless `extend`. |
| `pub fn select_all(&mut self)` | anchor = 0, caret = len. |
| `pub fn cut(&mut self) -> Option<String>` | `None` if no selection; else remove and return the selected text. |
| `pub fn set_caret(&mut self, at: usize, extend: bool)` | Clamp `at` to len, round down to a char boundary (loop on `is_char_boundary`; don't depend on `floor_char_boundary`), set caret; `anchor = caret` unless `extend`. |

Word helpers (private, pure): a word char is `c.is_alphanumeric() || c == '_'`.

- `fn word_left(text: &str, from: usize) -> usize` — step left over non-word chars, then over word chars; returns the start of that word (or 0).
- `fn word_right(text: &str, from: usize) -> usize` — step right over word chars, then over non-word chars; returns the start of the next word (or len). This is Notepad's Ctrl+Right.

Examples: in `"hello, world"`, `word_right(0) == 7`, `word_left(7) == 0`, `word_left(12) == 7`.

### 2.4 Key mapping

```rust
pub fn command(vk: u16, ctrl: bool, shift: bool) -> Option<Command>
```

The shell passes `ctrl = Ctrl down && Alt up` (see § 6.6); `command` itself knows nothing about Alt.

| Key (VK) | Condition | Command |
| --- | --- | --- |
| Left `0x25` / Right `0x27` | `ctrl` | `Move(WordLeft/WordRight, shift)` |
| Left / Right | `!ctrl` | `Move(Left/Right, shift)` |
| Home `0x24` / End `0x23` | any | `Move(Home/End, shift)` |
| Backspace `0x08` | any | `Backspace(ctrl)` |
| Delete `0x2E` | `shift && !ctrl` | `Cut` |
| Delete | otherwise | `Delete(ctrl)` |
| Insert `0x2D` | `ctrl` → `Copy`; `shift` → `Paste`; else `None` | |
| A `0x41` / C `0x43` / X `0x58` / V `0x56` | `ctrl` | `SelectAll` / `Copy` / `Cut` / `Paste` |
| Return `0x0D` | any | `Submit` |
| Escape `0x1B` | any | `Cancel` |
| anything else | | `None` |

Up, Down and Tab return `None`; Task 1-2 maps them in the shell.

### 2.5 `Utf16Joiner::push`

```rust
pub fn push(&mut self, unit: u16) -> Option<char>
```

- `0xD800..=0xDBFF` (high surrogate): remember it (replacing any earlier unpaired one); return `None`.
- `0xDC00..=0xDFFF` (low surrogate): if a high one is remembered, take it and return the combined char; else `None` (orphan dropped).
- Anything else: forget any remembered high surrogate; return the char unless it `is_control()` (this drops 0x08 Backspace, 0x09 Tab, 0x0D Enter, 0x1B Esc, 0x7F Ctrl+Backspace and Ctrl+letter codes, all of which `WM_KEYDOWN` already handled).

### 2.6 Drawing

```rust
pub fn draw(cv: &Canvas, field: &TextField, r: Rect, font: Font, placeholder: &str, caret_on: bool)
pub fn caret_at(cv: &Canvas, field: &TextField, r: Rect, font: Font, x: f32) -> usize

/// How far the text is shifted left so the caret stays visible: 0 until the caret passes
/// the right edge, then exactly enough to keep it there. Stateless.
fn scroll_offset(caret_x: f32, visible_w: f32) -> f32          // (caret_x - visible_w).max(0.0)
/// The char boundary whose prefix width is closest to `x` (first one on a tie).
fn nearest_boundary(text: &str, x: f32, measure: impl Fn(&str) -> f32) -> usize
```

Constants: `CARET_ROOM: f32 = 2.0` (so the caret isn't clipped at the right edge); `SELECTION: Rgb = Rgb(52, 84, 140)`.

`draw`, in order:

1. `visible_w = (r.w - CARET_ROOM).max(0.0)`; `caret_x = cv.measure(&text[..caret], font)`; `off = scroll_offset(caret_x, visible_w)`.
2. `cv.push_clip(r)` (`src/gfx.rs:733`). Clips nest inside `render`'s body clip.
3. Selection (if any): fill `Rect::new(r.x + x0 - off, r.y + 7.0, x1 - x0, r.h - 14.0)` with `Ink::Rgb(SELECTION)`, radius 2, where `x0`/`x1` are the measured prefix widths of the selection's ends.
4. Empty text: `cv.text(placeholder, font, Ink::Dim, r, Align::Left)`. Otherwise `cv.text(text, font, Ink::Text, Rect::new(r.x - off, r.y, full + CARET_ROOM + 2.0, r.h), Align::Left)` where `full = cv.measure(text, font)`. The rect must be at least as wide as the text: `Canvas::text` trims overflow with an ellipsis (`src/gfx.rs:401-407`, `520-538`), and the clip from step 2 does the cutting instead.
5. If `caret_on`: a 1 px `cv.line` at `x = r.x + caret_x - off`, from `r.y + 7` to `r.y + r.h - 7`, `Ink::Text`.
6. `cv.pop_clip()`.

`caret_at` recomputes `off` exactly as `draw` does (same field state ⇒ same offset), then returns `nearest_boundary(text, x - r.x + off, |s| cv.measure(s, font))`. `x` is in the same coordinate space as `r`. O(n) `measure` calls, n ≤ `max_chars`; it runs once per click.

Prefix widths come from `measure`, which uses `widthIncludingTrailingWhitespace` (`src/gfx.rs:546`), so a caret after a trailing space lands after it. Kerning across the caret may make the caret a fraction of a pixel off; acceptable.

### 2.7 Unit tests (`#[cfg(test)] mod tests` in `text.rs`)

Use a fake measure `|s: &str| s.chars().count() as f32 * 10.0` for the layout helpers.

- `insert_replaces_the_selection_and_sanitises_control_chars` — `"a\r\nb\tc\u{7}d"` inserts as `"a b cd"`; inserting over a selection replaces it and leaves the caret after the insert.
- `insert_respects_max_chars` — `new(5)`, insert `"abcdefg"` → `"abcde"`; with 3 selected of 5, inserting 10 chars keeps 3; inserting into a full field without a selection changes nothing.
- `backspace_and_delete_by_char_and_by_word` — single chars, word-wise from the middle of `"hello, world"`, no-ops at the ends, and a selection deleted regardless of `word`.
- `caret_motions_extend_and_collapse_the_selection` — Shift+Left twice selects two chars; Left without Shift collapses to the selection's start; Right to its end; Home/End with extend.
- `word_motions_skip_spaces_and_punctuation` — the `"hello, world"` examples above plus leading/trailing spaces.
- `indices_stay_on_char_boundaries_with_multibyte_text` — `"aé😀中b"`: every motion, backspace and delete leaves caret/anchor on boundaries and removes whole chars.
- `cut_and_selected_return_the_selection` — `cut()` is `None` without a selection, else returns and removes it.
- `set_caret_clamps_and_rounds_down_to_a_char_boundary` — `set_caret(99)` → len; an index inside `é` rounds down; `extend` keeps the anchor.
- `command_maps_editing_keys` — every row of § 2.4, including Shift+Delete → `Cut`, Ctrl+Insert → `Copy`, Shift+Insert → `Paste`, `command(0x41, false, false) == None`, Up (0x26) → `None`.
- `joiner_pairs_surrogates_and_drops_controls_and_orphans` — `D83D DE00` → `'😀'`; a lone low → `None`; high then `'a'` → `'a'`; 0x08, 0x09, 0x0D, 0x1B, 0x7F, 0x01 → `None`; `'é'` → `Some`.
- `scroll_offset_keeps_the_caret_visible` — `(50, 100)` → 0; `(150, 100)` → 50.
- `nearest_boundary_picks_the_closest_char_edge` — `"abcd"`: x 14 → 1, 16 → 2, −5 → 0, 100 → 4; `"aé😀b"` (boundaries 0, 1, 3, 7, 8): x 26 → 7.

## 3. `src/clipboard.rs`

Add, beside `Watcher` (after `src/clipboard.rs:138`):

```rust
/// The clipboard's text, or `None` if it has none, it's too big, or the clipboard is busy.
/// Not filtered for private copies: the user is pasting into our own field.
pub fn read_text(owner: HWND) -> Option<String>

/// Puts `text` on the clipboard as CF_UNICODETEXT, owned by `owner`. Returns whether it worked.
pub fn write_text(owner: HWND, text: &str) -> bool
```

- Both open the clipboard with `OpenGuard::new(owner)` (`src/clipboard.rs:161-181`, which retries 5 × 10 ms and closes on drop).
- `owner` must be a real window: with a NULL owner, `EmptyClipboard` leaves the clipboard ownerless and `SetClipboardData` then fails. The shell passes the notch's `hwnd`; `DefWindowProcW` handles the resulting `WM_DESTROYCLIPBOARD`.
- `read_text`: `IsClipboardFormatAvailable(CF_UNICODETEXT)`, then `read_global(CF_UNICODETEXT, MAX_TEXT * 2)` (`src/clipboard.rs:250-265`), then decode.
- `write_text`: `EmptyClipboard`, then CF_UNICODETEXT data.

Factor out, so the existing paths are unchanged in behaviour:

- `fn utf16_text(bytes: &[u8]) -> String` — the decode now inline at `src/clipboard.rs:216-218` (UTF-16LE up to the first NUL, lossy). `capture` keeps its `trim().is_empty()` check (219).
- `unsafe fn set_text(s: &str) -> bool` — the `Content::Text` arm of `Watcher::copy` (`src/clipboard.rs:120-124`). `Watcher::copy` calls it.

Our own `write_text` triggers the `Watcher`'s `WM_CLIPBOARDUPDATE` like any other copy, so field copies land in Clipboard history with no extra code (P1-D9).

Test: `utf16_text_stops_at_the_first_nul` — bytes for `"hi\0junk"` decode to `"hi"`; no NUL decodes the whole buffer.

## 4. `src/gfx.rs`

Add `pub const SEARCH: &str = "\u{E721}";` to `glyph` after `PALETTE` (`src/gfx.rs:366`). U+E721 is "Search" in both Segoe Fluent Icons and Segoe MDL2 Assets.

## 5. `src/main.rs`

`mod text;` between lines 13 and 14.

## 6. `src/app.rs`

### 6.1 Imports

- Line 18: add `GetKeyState, SetFocus, VK_CONTROL, VK_F4, VK_MENU, VK_SHIFT` to the `KeyboardAndMouse` list. `WindowsAndMessaging::*` (line 23) already brings `GetForegroundWindow`, `SetForegroundWindow`, `IsWindow`, `GetCaretBlinkTime`, `WM_KEYDOWN`, `WM_SYSKEYDOWN`, `WM_CHAR`, `WM_ACTIVATE`, `WA_INACTIVE`, `IDC_IBEAM`.
- After line 30: `use crate::clipboard;` and `use crate::text::{self, Command, TextField, Utf16Joiner};`.

### 6.2 Constants and geometry

With the layout constants (`src/app.rs:49-75`):

```rust
/// Longest search query, in chars.
const SEARCH_MAX: usize = 100;
/// Room for the magnifier glyph at the left of the open search field.
const FIELD_ICON: f32 = 32.0;
```

Replace `handle_rect` (`src/app.rs:118-123`) with these five functions. Values are for today's five widgets.

```rust
/// The empty stretch of the tab row between the widget tabs and the Settings tab. (312, 0, 304, 40)
fn stretch_rect(widgets: &[Box<dyn Widget>]) -> Rect   // today's handle_rect body, unchanged
/// The magnifier button at the stretch's right end, shaped like a tab.        (572, 8, 44, 32)
fn search_rect(widgets: &[Box<dyn Widget>]) -> Rect    // Rect::new(s.right() - TAB_W, TAB_Y, TAB_W, TAB_H)
/// While searching, the whole stretch is the search field.                     (312, 8, 304, 32)
fn field_rect(widgets: &[Box<dyn Widget>]) -> Rect     // Rect::new(s.x, TAB_Y, s.w, TAB_H)
/// Where the field's text goes, right of its glyph.                            (344, 8, 262, 32)
fn field_text_rect(widgets: &[Box<dyn Widget>]) -> Rect // Rect::new(f.x + FIELD_ICON, f.y, f.w - FIELD_ICON - 10.0, f.h)
/// The slide handle: the stretch left of the magnifier. Dragging it slides the notch. (312, 0, 256, 40)
fn handle_rect(widgets: &[Box<dyn Widget>]) -> Rect    // Rect::new(s.x, 0.0, search_rect(w).x - TAB_GAP - s.x, TAB_BASE)
```

Placement rationale: the button is carved from the stretch's right end so the tab row stays one line, the button sits beside the Settings tab, and the field can grow leftwards from it over the whole stretch. `handle_rect` excludes the button, so `over_handle` (↔ cursor) and the slide branch of `on_press` never fire over it.

### 6.3 Types

After `Modal` (`src/app.rs:305-308`):

```rust
/// Who holds the keyboard. Task 1-3 adds `Shortcut`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Owner {
    /// The tab-row search field.
    Search,
}

/// Keyboard focus the notch took after an explicit user action; see `begin_typing`.
struct Keyboard {
    owner: Owner,
    /// The window that had the foreground before, to give it back to.
    restore: Option<HWND>,
    /// Caret visibility, toggled at the system blink rate; solid right after each edit.
    caret_on: bool,
    blink_at: Instant,
}
```

### 6.4 `App` fields

Add before `dirty` (`src/app.rs:342`), and initialise in `App::new` (`src/app.rs:418-441`) as `None`, `None`, `Utf16Joiner::default()`:

```rust
/// Set while the notch holds the keyboard; see `begin_typing` / `end_typing`.
keyboard: Option<Keyboard>,
/// The search field, while search is open.
search: Option<TextField>,
/// Pairs UTF-16 surrogates from WM_CHAR.
joiner: Utf16Joiner,
```

`search.is_some()` ⇒ `keyboard` is `Some` with `owner == Search`, and vice versa (invariant F5 below).

### 6.5 Focus: `begin_typing`, `end_typing`, `open_search`

```rust
/// Takes keyboard focus for `owner`. Only call straight from an explicit user action: a
/// click on the notch (we received the last input event) or WM_HOTKEY. Returns false, and
/// holds nothing, if the notch did not become the foreground window.
fn begin_typing(&mut self, owner: Owner) -> bool
```

1. `fg = GetForegroundWindow()`. `restore` = the existing `keyboard`'s `restore` if typing is already under way (defensive; callers end typing first), else `Some(fg)` when `fg` is valid and `!= self.hwnd`, else `None`.
2. Set `self.keyboard = Some(Keyboard { owner, restore, caret_on: true, blink_at: now + blink })`, reset `self.joiner`, `self.dirty = true`. **All state is in place before step 3.**
3. `SetForegroundWindow(self.hwnd)`. `WS_EX_NOACTIVATE` stops click activation, not this call. It sends `WM_ACTIVATE` and `WM_SETFOCUS` synchronously; they re-enter `wndproc` while `APP` is borrowed and `with_app` drops them (`src/app.rs:991-999`). Nothing depends on them being handled. `DefWindowProcW` still runs for them (§ 6.7), so keyboard focus is set.
4. If `GetForegroundWindow() != self.hwnd`: set `self.keyboard = None`; return `false`. Otherwise `let _ = SetFocus(Some(self.hwnd));` (belt and braces) and return `true`.

```rust
/// Gives the keyboard up. `restore` hands the foreground back to the window that had it
/// (Esc, a tab click, the shortcut toggle); `false` leaves it where it's going (a click
/// elsewhere, a launched app).
fn end_typing(&mut self, restore: bool)
```

1. `let Some(kb) = self.keyboard.take() else { return };`
2. Owner cleanup: `Owner::Search` → `self.search = None`. Reset `self.joiner`; `self.dirty = true`.
3. Only now: if `restore`, `kb.restore` is `Some(prev)` and `IsWindow(Some(prev))`, call `SetForegroundWindow(prev)`. We are the foreground process at this point, so Windows allows it. The resulting `WM_ACTIVATE(WA_INACTIVE)` to the notch is dropped by `with_app`, which is harmless because step 2 already ran. Clearing first is what makes the dropped message harmless.

```rust
/// Opens the tab-row search with an empty query and takes the keyboard.
fn open_search(&mut self)
```

Sets `self.search = Some(TextField::new(SEARCH_MAX))`, then calls `begin_typing(Owner::Search)`; on `false`, sets `self.search = None`. `self.dirty = true`.

Helpers (private, free functions):

```rust
/// Whether `vk` is down, as of the message being handled.
fn key_down(vk: VIRTUAL_KEY) -> bool            // GetKeyState(vk) < 0
/// The caret blink period; `None` when the user turned blinking off (INFINITE) or it failed (0).
fn caret_blink() -> Option<Duration>            // GetCaretBlinkTime()
```

And a method `fn edited(&mut self)`: if typing, set `caret_on = true` and `blink_at = now + blink`; `self.dirty = true`.

### 6.6 Focus invariants

- F1. `begin_typing` is called only from `on_press` (a click on the magnifier; Task 1-3 adds the shortcut chip) and from `WM_HOTKEY` (Task 1-3). Never from `tick`, hover, expansion, peek, `select_tab` or a widget hook.
- F2. Hover, expansion, tab clicks, widget clicks and the pin never call `SetForegroundWindow` or `SetFocus`. `WM_MOUSEACTIVATE` keeps returning `MA_NOACTIVATE` (`src/app.rs:1154`).
- F3. Every path that ends typing goes through `end_typing`. `restore = true`: Esc, a tab click, Alt+F4 while typing (and Task 1-3's toggle and recorder outcomes). `restore = false`: `WM_ACTIVATE(WA_INACTIVE)`, the `tick` safety net, a launch (Task 1-2).
- F4. Typing state is final before any call that can re-enter `wndproc` (`SetForegroundWindow`, `SetFocus`, clipboard calls). No behaviour depends on `WM_ACTIVATE`, `WM_SETFOCUS` or `WM_KILLFOCUS` reaching `App` during such a call.
- F5. `search.is_some()` ⇔ `keyboard.as_ref().map(|k| k.owner) == Some(Owner::Search)`, outside the body of `open_search`/`end_typing`.
- F6. While `keyboard.is_some()` the notch's open target is 1 (§ 6.8), so it can't collapse while holding focus.
- F7. While `keyboard.is_some()`, the notch is the foreground window at the start of every frame; otherwise `tick` ends typing without restoring.

### 6.7 Input routing

**`on_press`** (`src/app.rs:665-719`). Keep the `t < 0.9` guard (666) and the PIN branch (672-673). Then, in this order:

1. Tab hit (674-675): call `self.end_typing(true)` (a no-op when not typing), then `self.select_tab(t)` (P1-D4).
2. `else if self.search.is_some()`: if `field_rect` contains the point, place the caret: compute `at = text::caret_at(&cv, field, field_text_rect(..), Font::Body, lx)` with a measuring canvas `Canvas::new(&self.rt, &self.brushes, &self.layout.fonts, &self.images, s, (0.0, 0.0), None)` (measure needs only fonts and scale), then `field.set_caret(at, key_down(VK_SHIFT))` and `self.edited()`. Any other point does nothing while searching. Content clicks are left for Task 1-2. Slide-drag, Settings and widget clicks must not run.
3. `else if search_rect(..).contains(lx, ly)`: `self.open_search()`.
4. The existing handle (676-679), Settings (680-697) and widget (698-717) branches, unchanged.

**`on_key(&mut self, vk: u16, sys: bool) -> bool`** (true = handled, don't pass on):

- Not typing → `false`.
- `sys` (`WM_SYSKEYDOWN`): if `vk == VK_F4.0`, `end_typing(true)` and return `true`. `DefWindowProcW` would turn Alt+F4 into `SC_CLOSE` and quit MyNotch. Otherwise `false`.
- `ctrl = key_down(VK_CONTROL) && !key_down(VK_MENU)`: AltGr arrives as Ctrl+Alt and types characters on many layouts, so a Ctrl+Alt chord is never an editing command. `shift = key_down(VK_SHIFT)`.
- `Owner::Search`: `text::command(vk, ctrl, shift)`; `None` → return `false`. Otherwise apply it and return `true`:

| Command | Effect |
| --- | --- |
| `Move`, `Backspace`, `Delete`, `SelectAll` | The matching `TextField` method. |
| `Copy` | If the selection isn't empty, `clipboard::write_text(self.hwnd, field.selected())`. |
| `Cut` | If the selection isn't empty and `write_text` succeeds, `field.cut()`. If the clipboard is busy, the text stays (never lose text). |
| `Paste` | `if let Some(s) = clipboard::read_text(self.hwnd) { field.insert(&s) }`. |
| `Submit` | Nothing in Task 1-1 (Task 1-2 launches the highlighted result). |
| `Cancel` | `end_typing(true)` (P1-D3: closes and clears). |

  After any command except `Cancel`, call `self.edited()`.

**`on_char(&mut self, unit: u16) -> bool`**: not typing → `false`. Else `self.joiner.push(unit)`; for `Some(c)` and `Owner::Search`, `field.insert(c.encode_utf8(&mut [0; 4]))` and `self.edited()`. Return `true` (characters are swallowed while typing even when the joiner yields nothing).

**`wndproc`** (`src/app.rs:1096-1173`); add before the `WM_MOUSEACTIVATE` arm (1154):

```rust
WM_KEYDOWN | WM_SYSKEYDOWN => {
    let mut handled = false;
    with_app(|a| handled = a.on_key(wparam.0 as u16, msg == WM_SYSKEYDOWN));
    if handled { LRESULT(0) } else { unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) } }
}
WM_CHAR => { /* same shape, with a.on_char(wparam.0 as u16) */ }
// The user went elsewhere: stop typing and leave the focus where it went.
WM_ACTIVATE => {
    if (wparam.0 & 0xFFFF) as u32 == WA_INACTIVE {
        with_app(|a| a.end_typing(false));
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}
```

`WM_ACTIVATE` must reach `DefWindowProcW`; it is what gives the activated window keyboard focus. The class is registered with `RegisterClassW` (`src/app.rs:359-366`), so `WM_CHAR` carries UTF-16 code units.

**Cursor** (`WM_SETCURSOR`, `src/app.rs:1123-1132`): `over_handle` (`525-529`) gains `self.search.is_none() &&`. Add `over_field()` (expanded, searching, cursor in `field_rect`) and show `IDC_IBEAM` there. Keep the existing `IDC_SIZEWE` path.

### 6.8 `tick` (`src/app.rs:549-662`)

- After the peek expiry (567-569), add the safety net and the blink:
  - If `keyboard.is_some()` and `GetForegroundWindow() != self.hwnd`: `end_typing(false)` (F7). This catches a `WA_INACTIVE` that `with_app` dropped, for example one delivered while a clipboard call was waiting on another process.
  - If typing, `caret_blink()` is `Some(p)` and `now >= blink_at`: flip `caret_on`, `blink_at = now + p`, `dirty = true`.
- Line 642: add `|| self.keyboard.is_some()` to the target (F6).

### 6.9 Drawing

**`draw_tabs`** (`src/app.rs:898-941`):

- 903-906: the baseline gap goes under `field_rect` while searching, else under the active tab.
- 915: a tab is drawn active only if `t == self.active && self.search.is_none()`.
- 924-933, while searching: `cv.tab(field_rect, TAB_RADIUS, Some(Ink::Track), Some(Ink::Edge))` (the active tab's look, line 916); `glyph::SEARCH` in `Font::Icon`, `Ink::Dim`, centred in the field's left `FIELD_ICON` px; `text::draw(cv, field, field_text_rect, Font::Body, "Search apps", caret_on)`. No name label and no grip.
- 924-933, otherwise: the name label and grip as today, inside the narrowed `handle_rect` (the label is `handle.w / 2 − 20` = 108 px; the grip's centre moves to x 440). The label shows "Search" while the magnifier is hovered, else today's tab name. Then the magnifier drawn like an unselected tab (909-922): `cv.tab(search_rect, TAB_RADIUS, hot.then_some(Ink::Track), Some(Ink::Line))` with `glyph::SEARCH` in `Ink::Text` when hovered, else `Ink::Dim`.

**`render`** (`src/app.rs:845-856`): before the Home branch, while `self.search.is_some()`, draw `"Type to search apps"` in `Font::Body`, `Ink::Dim`, centred in `Rect::new(0.0, 0.0, CONTENT.w, CONTENT.h)`, and nothing else in the content area.

### 6.10 Geometry test

In `src/app.rs`'s test module (`1175+`), add `search_button_and_field_share_the_tab_row_stretch`. Build five boxed test widgets: a private `struct Dummy` implementing only the five required `Widget` methods. Don't use `widgets::all()`, which spawns the media worker and a clipboard window. Assert the five rects in § 6.2 exactly, and that `handle.right() + TAB_GAP == search.x`, `search.right() == field.right()`, and `field.right() + TAB_GAP == tab_rect(settings).x`.

## 7. Edge cases

- **The notch is foreground but not typing** (after the menu's grab, or after a launch). Keys and chars pass to `DefWindowProcW`; nothing is typed. Alt+F4 then still quits MyNotch, as it does today (out of scope).
- **The search button clicked mid-animation.** `on_press` ignores clicks until `t ≥ 0.9` (666), as today.
- **Right-click while searching.** `show_menu` calls `SetForegroundWindow(hwnd)` (1041), a no-op since we're foreground. Observe at runtime that search is either still open with the notch foreground, or closed; never "held but not foreground" (F7).
- **Paste of a huge clipboard.** `read_global` refuses blocks over `MAX_TEXT * 2` bytes (`src/clipboard.rs:37`, `254`), so the paste does nothing; smaller ones are cut to `SEARCH_MAX` chars.
- **Clipboard busy.** `OpenGuard` waits up to ~50 ms, then Copy/Cut/Paste do nothing. Cut never deletes without a successful write.
- **IME.** Committed text arrives as `WM_CHAR` and is inserted; the composition window's position is not managed (non-goal).
- **Previous window closed meanwhile.** `IsWindow` fails and nothing is restored; the notch stays foreground until the user clicks elsewhere.

## 8. Verification

### 8.1 Build and unit tests

```powershell
cargo build
cargo clippy --all-targets
cargo test
```

Expect no clippy warnings and 53 passing tests (39 + 12 in `text.rs` + 1 clipboard + 1 app).

### 8.2 Runtime (G1)

Write the scripts in your session scratchpad (fallback `Join-Path $env:TEMP 'mynotch-phase1'`), never in the repo.

**Setup**

1. Back up `%APPDATA%\MyNotch\settings.ini` and the clipboard text (`Get-Clipboard -Raw`; note in the report if the clipboard held non-text, which this can't restore). Record the cursor position.
2. Quit the running release instance (`FindWindow('MyNotchWindow')` + `PostMessage WM_CLOSE`, then `Wait-Process`), and launch `target\debug\mynotch.exe`.
3. Start the focus-test window in a separate process. It must log its text box to a file so you can tell where keystrokes went:

```powershell
$log = Join-Path $scratch 'focus-test.txt'
$form = Start-Process pwsh -PassThru -ArgumentList '-NoProfile', '-Command', @"
Add-Type -AssemblyName System.Windows.Forms
`$f = New-Object Windows.Forms.Form; `$f.Text = 'MyNotch focus test'; `$f.Width = 600; `$f.Height = 300
`$f.StartPosition = 'CenterScreen'
`$t = New-Object Windows.Forms.TextBox; `$t.Multiline = `$true; `$t.Dock = 'Fill'; `$f.Controls.Add(`$t)
`$t.Add_TextChanged({ Set-Content -Path '$log' -Value `$t.Text })
[Windows.Forms.Application]::Run(`$f)
"@
```

4. Helpers via `Add-Type`: `FindWindow`, `GetWindowRect`, `GetDpiForWindow`, `GetForegroundWindow`, `SetCursorPos`/`GetCursorPos`, `GetCursorInfo`, `LoadCursor`, `SendInput`. On x64 `INPUT` is 40 bytes with the union at offset 8; pass `Marshal.SizeOf` of your struct. Keys: virtual-key events (add `KEYEVENTF_EXTENDEDKEY` for arrows, Home, End, Insert, Delete); text: `KEYEVENTF_UNICODE` with one down/up pair per UTF-16 unit, so "😀" goes as two units. Mouse: `SetCursorPos`, then `SendInput` left down/up.
5. Coordinates. `scale = GetDpiForWindow(notch) / 96` (1.0 on the dev machine). The expanded layout's origin is `(rect.left + 18·scale, rect.top)` (`SHADOW`, `src/app.rs:56`; `Layout::place`, `264-270`). Logical points: magnifier centre (594, 24); field text start x 344; tab t centre (46 + 48·t, 24); Settings tab (642, 24); pin (684, 24); content origin (24, 48).
6. Screenshots: `System.Drawing.Graphics.CopyFromScreen` of the window rect, saved as PNGs in the scratchpad and inspected.
7. To read the field's exact text: Ctrl+A, Ctrl+C, then `Get-Clipboard -Raw`.

**Checks** (each records the actual result)

| # | Steps | Expect |
| --- | --- | --- |
| R1 | Click the test form. Move the cursor onto the notch and wait 600 ms. Click the System tab (t = 5), then the pin twice. | Notch expands; foreground stays the test form throughout. |
| R2 | Hover the stretch at x 400, then the magnifier. Screenshot. | ↔ over x 312–568, arrow over the magnifier; the label reads "Search" over the button; the grip is centred at x 440. |
| R3 | Click the magnifier. | Within 200 ms the foreground is the notch. Screenshot: outlined field across x 312–616 joined to the panel; no widget tab active; "Search apps" placeholder; "Type to search apps" in the content. |
| R4 | Type `hello world`; read the field. | `hello world`; `focus-test.txt` unchanged. |
| R5 | End, Ctrl+Backspace; read. | `hello `. |
| R6 | Type `wörld 😀`; read. | `hello wörld 😀` (surrogate pair joined). |
| R7 | Home, Ctrl+Right, Shift+End; screenshot; Ctrl+X; `Get-Clipboard`; then read the field. | The selection is highlighted; the clipboard holds `wörld 😀`; the field holds `hello `. |
| R8 | Home, Ctrl+V; read. | `wörld 😀hello `. |
| R9 | `Set-Clipboard "line one`r`nline two"`; Ctrl+A, Ctrl+V; read. | `line one line two`. |
| R10 | Ctrl+A, type `abc`, Left, Left, Delete, read; Backspace, read. | `ac`, then `c`. |
| R11 | Ctrl+A, type `abc`, Ctrl+Alt+A, type `d`; read. | `abcd` (Ctrl+Alt is not Select All). |
| R12 | Ctrl+A, type `abcdefghij`; click at field x 344 + 30; type `X`; read. Then click near the text's start, Shift+click near its end, type `Y`; read. | `X` lands inside the text, not at either end. Then most of the text is replaced by `Y`. |
| R13 | Press in the field and drag 100 px right; release. | The cursor shows the I-beam; the window rect and `position=` in settings.ini are unchanged. |
| R14 | Esc. Then send `zz` as Unicode input. | The foreground is the test form within 200 ms; `focus-test.txt` ends with `zz`; the notch collapses once the cursor leaves. |
| R15 | Open search, type `abc`, click the Media tab (t = 1). | The foreground is the test form; Media is active; settings.ini has `tab=Media`; the field is gone. |
| R16 | Open search, then click the test form. | The foreground is the test form and stays so for 1 s; on hover, search is closed. |
| R17 | Open search, send Alt+F4. | Search closes; the foreground is the test form; `Get-Process mynotch` still lists the dev exe. |
| R18 | Open search, move the cursor well away, wait 1 s, screenshot; Esc. | Still expanded with the field; it collapses after Esc. |
| R19 | Open search, right-click the notch, Esc out of the menu; check focus and screenshot. | Either search is still open with the notch foreground, or it's closed. Never open while another window is foreground. Record which. |
| R20 | Open the Clipboard tab. | The copies from R7 (and R9's setup) are listed (P1-D9). |
| R21 | Optional: two screenshots half a blink period apart while idle in the field. | The caret is visible in one and not the other. |

**Cleanup.** Close the test form (`Stop-Process $form`). Quit the dev MyNotch. Restore settings.ini and the clipboard text. Restore the cursor. If the session ends here, relaunch `target\release\mynotch.exe` and confirm it runs.

Report every check's actual outcome, the screenshot paths, and anything skipped with the reason. Don't claim a check you didn't run.
