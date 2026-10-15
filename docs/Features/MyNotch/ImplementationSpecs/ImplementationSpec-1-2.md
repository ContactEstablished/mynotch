# Implementation spec 1-2 — Search hooks and Apps results

Companion: [Task 1-2](../Tasks/Task-1-2.md). Overview: [Phase 1](../Tasks/Phase-1-Overview.md). Builds on [ImplementationSpec-1-1](ImplementationSpec-1-1.md). Line numbers in `src/widgets/` are from `63f6951` (Task 1-1 doesn't touch those files). In `src/app.rs` they're baseline numbers that Task 1-1 has shifted; find code by the function names given.

## 1. Files and boundaries

| File | Change |
| --- | --- |
| `src/widgets/mod.rs` | `SearchKey`, `SearchAction`, five defaulted hooks on `Widget`. |
| `src/widgets/apps.rs` | Search state, pure ranking/results/follow functions, result drawing and hooks, tests. |
| `src/app.rs` | Routing only. No new focus calls; launching uses Task 1-1's `end_typing(false)`. |

## 2. `src/widgets/mod.rs`

### 2.1 Enums

After `Compact` (`src/widgets/mod.rs:70-77`):

```rust
/// Keys the shell forwards to the widget showing search results.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchKey {
    /// Down or Tab: the next result in reading order.
    Next,
    /// Up or Shift+Tab.
    Previous,
    /// Open the highlighted result.
    Enter,
}

/// What a search key or click did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchAction {
    None,
    /// The results changed; redraw.
    Redraw,
    /// A result was opened (e.g. an app launched): the shell closes search without giving
    /// focus back, so the opened thing can come to the front.
    Done,
}
```

### 2.2 Trait hooks

At the end of the `Widget` trait, after `click_card` (`src/widgets/mod.rs:144-148`) and before the closing brace at 149:

```rust
/// The search query changed; also called with "" whenever search opens. Returns true when
/// this widget offers results for it. The shell shows the first widget (in `all()` order)
/// that does. There is no end-of-search call: state kept for a search goes stale when it
/// closes, and the next `search` call replaces it.
fn search(&mut self, _query: &str) -> bool { false }

/// Draws this widget's results over the content area, `w` x `h`, at the canvas origin.
fn draw_search(&self, _cv: &Canvas, _w: f32, _h: f32) {}

/// A navigation key while this widget's results are shown.
fn search_key(&mut self, _key: SearchKey) -> SearchAction { SearchAction::None }

/// A left click on the results, in content-area coordinates.
fn click_search(&mut self, _x: f32, _y: f32, _w: f32, _h: f32) -> SearchAction { SearchAction::None }

/// Mouse wheel over the results, in notches. Returns true when it needs a redraw.
fn wheel_search(&mut self, _notches: f32) -> bool { false }
```

Rationale: they sit beside the other per-surface hook groups (panel, Home card). Defaults keep Media, Time, Clipboard and System unchanged. `search` takes the raw field text; each widget normalises it as it needs.

## 3. `src/widgets/apps.rs`

### 3.1 Pure functions (module level, private, unit-tested)

```rust
/// How well `name` matches `query`, both compared lower-cased: 0 = the name starts with it,
/// 1 = a word in the name starts with it, 2 = the name's initials start with it
/// ("vsc" → Visual Studio Code), 3 = it appears anywhere; None = no match.
fn rank(name: &str, query: &str) -> Option<u8>

/// Search results as indices into `apps`, best first. A blank (all-whitespace) query lists
/// the installed pinned apps in pinned order. Otherwise every app that `rank`s, ordered by
/// tier, then pinned before unpinned, then shorter name (in chars), then name (lower-cased),
/// then list order.
fn results(apps: &[AppEntry], pinned: &[String], query: &str) -> Vec<usize>

/// The first visible row that keeps `row` on screen with `rows` rows showing, moving as
/// little as possible from `scroll`.
fn follow(scroll: usize, row: usize, rows: usize) -> usize
```

Definitions, all on `name.to_lowercase()` and `query.trim().to_lowercase()`:

- A **word** is a maximal run of `char::is_alphanumeric` chars. A **word start** is the byte index of a word's first char. Camel case does not start a word ("OneNote" has one word).
- Tier 0: `name.starts_with(query)`.
- Tier 1: some word start `i > 0` with `name[i..].starts_with(query)`. The query may run past the word, so "studio c" matches "Visual Studio Code".
- Tier 2: the string of each word's first char starts with `query` ("Paint 3D" → "p3").
- Tier 3: `name.contains(query)`.
- Lower-casing may change byte lengths (e.g. "İ"), so compute word starts on the lower-cased string, never on the original.

`results` with a non-blank query sorts with a stable sort on the key `(tier, !pinned, name.chars().count(), name.to_lowercase())`. The stable sort keeps list order for exact ties; the list is already sorted by name (`src/apps.rs:58`). With a blank query it maps `pinned` ids to indices of installed apps, in pinned order, skipping ids that aren't installed (as `shown()` does, `src/widgets/apps.rs:121-124`).

`follow`: `row < scroll` → `row`; `row >= scroll + rows` → `row + 1 - rows`; else `scroll`.

### 3.2 State

```rust
/// The search the shell last asked for; see `Widget::search`. Left as it is when search
/// closes; the next `search` call replaces it.
struct Search {
    /// Trimmed, as typed.
    query: String,
    /// Indices into `apps`, best match first.
    results: Vec<usize>,
    /// The highlighted result: an index into `results`.
    hit: usize,
    /// First visible row of results.
    scroll: usize,
}
```

`AppsWidget` (`src/widgets/apps.rs:85-99`) gains `search: Option<Search>` after `grab`; `new()` (101-114) sets it to `None`.

Invariants: `hit < results.len()` whenever `results` isn't empty (else `hit == 0`); `scroll <= result_max_scroll`; every index in `results` is `< apps.len()`. `apps` only changes once, in `update`, and that recomputes `results` (§ 3.4), so indices stay valid.

### 3.3 Drawing helpers (refactors that keep the grid pixel-identical)

- `fn draw_face(&self, cv: &Canvas, app: &AppEntry, r: Rect)` — the icon and the label from `draw_tile`'s `Tile::App` arm (`src/widgets/apps.rs:139-147`). `draw_tile` calls it, then draws its badges (148-153) as now.
- `fn cell(w: f32, i: usize, scroll: usize) -> Rect` — grid slot `i` with `scroll` rows hidden: the arithmetic in `layout` (184-185, 192-193). `layout` uses it.
- `fn draw_scrollbar(cv: &Canvas, w: f32, h: f32, scroll: usize, max: usize)` — `draw`'s scrollbar block (259-268), drawn only when `max > 0`. `draw` uses it.
- `fn result_layout(&self, s: &Search, w: f32, h: f32) -> Vec<(usize, Rect)>` — the visible results as (position in `results`, cell rect), for positions in `s.scroll * cols .. (s.scroll + rows) * cols` that exist.
- `fn result_max_scroll(&self, s: &Search, w: f32, h: f32) -> usize` — `results.len().div_ceil(cols).saturating_sub(rows)`.

### 3.4 Hook implementations (in `impl Widget for AppsWidget`)

**`search(query)`** — `self.search = Some(Search { query: query.trim().to_owned(), results: results(&self.apps, pinned, trimmed), hit: 0, scroll: 0 })`, with `pinned = self.pinned.as_deref().unwrap_or(&[])`. Returns `true` always, including before the list loads, so Apps owns the content area in Phase 1.

**`update`** (212-231) — in the `Loaded::List` arm, after `self.apps` and `self.pinned` are set (219-223), if `self.search` is `Some`, recompute its `results` from its `query` and reset `hit` and `scroll` to 0. (`changed` is already true.)

**`draw_search(cv, w, h)`**:

1. `self.size.set((w, h))`, as `draw` does (238), so keys and wheel know the grid.
2. Not loaded → "Loading apps…" exactly as at 240.
3. No results: a blank query → "Type to search apps"; otherwise `format!("No apps match \"{query}\"")`. Both `Font::Body`, `Ink::Dim`, centred in `Rect::new(0.0, 0.0, w, h)`. A long query is trimmed by `Canvas::text`'s ellipsis.
4. Otherwise, for each `(pos, r)` in `result_layout`: fill `r` (radius 12, as at 137) with `Ink::Hover` when `pos == hit`, else `Ink::Track` when `cv.hovered(r)`, else nothing; then `draw_face`. No unpin badge, no ✓.
5. `draw_scrollbar(cv, w, h, s.scroll, result_max_scroll)`.

**`search_key(key)`** — with `(w, h) = self.size.get()`, `cols = columns(w)`, `rows = visible_rows(h)`:

- `Enter`: if `results.get(hit)` is `Some(&i)`, `apps::launch(&self.apps[i].id)` and return `Done`; else `None`.
- `Next` / `Previous`: no results → `None`. Else `hit = (hit + 1).min(len - 1)` or `hit.saturating_sub(1)` (no wrap-around), `scroll = follow(scroll, hit / cols, rows)`. Return `Redraw` if either changed, else `None`.

**`click_search(x, y, w, h)`** — `self.size.set((w, h))`. If a cell from `result_layout` contains the point, launch that app and return `Done`; else `None`. It launches on button-down (the shell calls it from `WM_LBUTTONDOWN`). There's no drag on results, so there's no reason to wait for the release, unlike the pinned grid (`press`/`release`, 313-325, 349-355).

**`wheel_search(notches)`** — as `wheel` (357-363) but over `result_max_scroll`. It moves `scroll` only, not `hit`; returns whether `scroll` changed.

The existing `draw`, `click`, `press`, `drag`, `release` and `wheel` do not look at `search`.

### 3.5 Tests

Extend the test module's imports (`src/widgets/apps.rs:380`) with `follow`, `rank`, `results` and `crate::apps::AppEntry`.

- `rank_tiers_follow_the_name_words_and_initials` — for "Visual Studio Code": "vis" → 0, "code" → 1, "studio c" → 1, "vsc" → 2, "sual" → 3, "vsx" → None. Also "Paint 3D" + "3d" → 1, "OneNote" + "note" → 3.
- `rank_is_case_insensitive_and_handles_non_ascii` — "Microsoft Edge" + "EDGE" → 1; "Éclair Notes" + "écl" → 0; "Éclair Notes" + "ÉN" → 2 (initials "én"; plain "en" does not match them, since é ≠ e).
- `blank_query_lists_installed_pins_in_order` — apps `calc`/Calculator, `np`/Notepad, `vs`/Visual Studio Code; pinned `["vs", "gone", "calc"]` → the indices of `vs` then `calc`, for both `""` and `"   "`; no pins → empty.
- `results_order_by_tier_then_pinned_then_length_then_name` — apps Notepad (`np`), Notepad++ (`npp`), OneNote (`on`), Sticky Notes (`sn`), in any order: "note" with no pins → np, npp, sn, on; with `npp` pinned → npp, np, sn, on. Also "Maps" and "Mail" given in that order, query "ma" → Mail, Maps.
- `follow_keeps_the_highlight_on_screen` — `follow(0, 3, 2) == 2`, `follow(5, 3, 2) == 3`, `follow(2, 3, 2) == 2`, `follow(0, 0, 2) == 0`.

## 4. `src/app.rs` — routing

### 4.1 State

- Import (`src/app.rs:34` at baseline): `use crate::widgets::{self, Compact, SearchAction, SearchKey, Widget};`.
- `App` gains `results: Option<usize>` ("the widget showing search results, by index into `widgets`, while searching"), next to Task 1-1's `search`; it starts as `None`.
- Invariant: `results.is_some()` ⇒ `search.is_some()`. `end_typing`'s `Owner::Search` cleanup sets `self.results = None` together with `self.search = None`, before any focus call (Task 1-1 F4).

### 4.2 `query_changed`

```rust
/// Tells every widget the query changed (also used with "" when search opens) and shows the
/// first that offers results.
fn query_changed(&mut self)
```

It reads the query from `self.search` (empty if `None`), sets `self.results = None`, calls `search(&query)` on **every** widget in order, and keeps the first index that returned `true`. Calling all of them keeps each widget's state current. It sets `dirty`.

Call sites:

- `open_search`: after `begin_typing` succeeds. The empty query then shows the pinned apps (P1-D5).
- After an editing command (`on_key`) or an inserted char (`on_char`), when the field's text differs from before. Compare the text before and after (at most 100 chars). Caret moves, selection changes, Copy, and a Cut or Paste that did nothing must not re-run the search, so the highlight survives them.

### 4.3 Keys

In `on_key`'s `Owner::Search` branch, before `text::command`:

| Key | Condition | Action |
| --- | --- | --- |
| Down (`VK_DOWN`) | any | `search_key(SearchKey::Next)` |
| Up (`VK_UP`) | any | `search_key(SearchKey::Previous)` |
| Tab (`VK_TAB`) | `!ctrl` | `Previous` with Shift, else `Next` |

Each returns `true` (handled). The joiner already drops the Tab `WM_CHAR` (0x09). `Command::Submit` (Enter) now calls `search_key(SearchKey::Enter)`. Left/Right stay with `text::command` (P1-D7).

```rust
/// Forwards a key to the widget showing results.
fn search_key(&mut self, key: SearchKey)
/// Acts on what a results widget did: Redraw → dirty; Done → end_typing(false) (P1-D8).
fn search_action(&mut self, action: SearchAction)
```

`search_key` does nothing when `results` is `None`.

### 4.4 Clicks, wheel, drawing

- `on_press`, in Task 1-1's `self.search.is_some()` branch, after the field check: if `CONTENT` contains the point and `results` is `Some(i)`, call `search_action(self.widgets[i].click_search(lx - CONTENT.x, ly - CONTENT.y, CONTENT.w, CONTENT.h))`. Widget `press`/`click` and Settings hits stay unreachable while searching.
- `on_wheel` (baseline `src/app.rs:788-802`): after the `t < 0.9` check, and before the `HOME` early return (789), add: while searching, if `results` is `Some(i)` and `wheel_search(notches)` is true, set `dirty`; then return.
- `render` (baseline 845-856): while searching, `Some(i)` → `self.widgets[i].draw_search(&content, CONTENT.w, CONTENT.h)`; `None` → Task 1-1's "Type to search apps" hint.

### 4.5 Launch and focus

`SearchAction::Done` → `end_typing(false)`. Order inside the shell: the widget has already called `apps::launch` (it spawns a thread that calls `ShellExecuteW`, `src/apps.rs:160-166`), and then the shell ends typing. At that moment the notch's process is the foreground process (keyboard path) or received the last input (click path), so Windows lets the launched app take the foreground. Restoring the previous window would race it, which is why P1-D8 says not to. Until the app's window appears, the notch may stay foreground while not typing; keys then go to `DefWindowProcW` (Task 1-1 § 7).

## 5. Edge cases

- **Search opened before the list loads** (more likely with Task 1-3's shortcut right after login): "Loading apps…"; when `Loaded::List` arrives, `update` recomputes for the current query and the frame redraws. Enter before then does nothing.
- **No pinned apps and a blank query**: "Type to search apps".
- **Query of only spaces**: trimmed to blank, so it shows the pins.
- **Results shrink while the highlight is far down**: every query change resets `hit` and `scroll` to 0.
- **The window is resized or the DPI changes while searching**: the grid uses the content size cached at the last draw, as the pinned grid does.
- **Two apps with the same name**: both are listed; the stable sort keeps list order.

## 6. Verification

### 6.1 Build and unit tests

```powershell
cargo build
cargo clippy --all-targets
cargo test
```

Expect no warnings and 58 passing tests.

### 6.2 Runtime (G1)

Same setup as ImplementationSpec-1-1 § 8.2: back up settings.ini and the clipboard text; quit the release instance; run `target\debug\mynotch.exe`; start the "MyNotch focus test" form; build the same input, foreground and screenshot helpers. In addition, before any launch, record the PIDs of the app you'll launch (e.g. `Get-Process CalculatorApp -ErrorAction SilentlyContinue`). Afterwards stop only processes that weren't there before. Never close windows the user had open.

| # | Steps | Expect |
| --- | --- | --- |
| S1 | Screenshot the Apps tab's pinned grid. Open search (magnifier). Screenshot. | The same apps in the same order, without × or ✓; the first one has the stronger fill; the notch is foreground. |
| S2 | Type `calc`. Screenshot. | Calculator is first and highlighted. |
| S3 | Ctrl+A, type the initials of an installed multi-word app (e.g. `vsc` if Visual Studio Code is listed; pick one from S1 or the picker). | That app is in the results, ahead of any tier-3 substring matches. |
| S4 | Ctrl+A, type `e` (many matches). Press Down 9 times; screenshot. Press Down until the highlight is on row 3; screenshot. Press Up past the top; Tab, Shift+Tab. | The highlight moves one result per press in reading order; the grid scrolls only when the highlight leaves rows 1–2; Up stops at the first result. |
| S5 | With the field holding `e` and a result highlighted, press Left; screenshot. Type `x`; read the field (Ctrl+A, Ctrl+C). Then Ctrl+A, type `e`. | Left moved the caret, not the highlight (the screenshot shows the same result highlighted). The field reads `xe`, and that text change reset the highlight to the first result. |
| S6 | Wheel down over the results; screenshot. | The results scroll; the highlight stays where it was. |
| S7 | Ctrl+A, type `zzqqxx`. Screenshot. | `No apps match "zzqqxx"`. |
| S8 | Click the test form. Open search, type `calc`, press Enter. Wait up to 3 s. | Search closes; a new Calculator window appears and becomes foreground (check its window title, since Store apps are hosted by ApplicationFrameHost); the test form is not foreground. Close only the new Calculator. |
| S9 | Click the test form. Open search, type `calc`, click the Calculator tile. | As S8. Close only the new Calculator. |
| S10 | Open search, type `abc`, Esc. Then open search, click the Media tab. | Task 1-1's R14 and R15 still hold. |
| S11 | Open the Apps tab. Hover a pinned app; open the picker; close it. Screenshot each. | The × badge, the picker's ✓, and the layout match S1's "before" screenshot pixel-for-pixel where nothing should differ. |

**Cleanup.** Close the test form, quit the dev MyNotch, restore settings.ini and the clipboard, restore the cursor, and confirm no test-launched app is still running. If the session ends here, relaunch `target\release\mynotch.exe`.

Report each check's actual result with screenshot paths. Don't claim a check you didn't run.
