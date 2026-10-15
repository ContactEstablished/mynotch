# Task 1-2 — App search results

Status: **ready; not started.** Depends on Task 1-1. Paired spec: [ImplementationSpec-1-2](../ImplementationSpecs/ImplementationSpec-1-2.md).

## Source Of Truth

- [Phase 1 overview](Phase-1-Overview.md): decisions P1-D5, P1-D6, P1-D7, P1-D8, P1-D12; gates G1, G2, G3, G7.
- [roadmap](../roadmap.md), Phase 1 goal 5 (as amended: search is the tab-row bar). [HANDOFF.md](../../../../HANDOFF.md): verification method.
- [ImplementationSpec-1-2](../ImplementationSpecs/ImplementationSpec-1-2.md), plus the names Task 1-1 defined ([ImplementationSpec-1-1](../ImplementationSpecs/ImplementationSpec-1-1.md) §§ 2, 6).

## Initial Starting Point

Expected after Task 1-1 (confirm before starting):

- `src/text.rs` exists. `src/app.rs` has `Owner::Search`, `Keyboard`, `App.keyboard`/`search`/`joiner`, `begin_typing`, `end_typing`, `open_search`, `on_key`, `on_char`, `search_rect`, `field_rect`, and the "Type to search apps" content hint. Enter in the field does nothing. `cargo test`: 53 passing.
- `src/app.rs` line numbers have moved since `63f6951`; find code by function name.

Unchanged by Task 1-1 (verified at `63f6951`, 2026-10-15):

- `src/widgets/mod.rs`: `Compact` at 70-77; the `Widget` trait at 79-149, whose last hook is `click_card` (144-148); `all()` at 151-160 (Apps is the third widget). The trait has no search or keyboard hooks.
- `src/widgets/apps.rs`: tile constants at 14-16 (84 × 88; on the 672 × 176 content area that gives 8 columns and 2 rows); `Tile` at 44-51; `AppsWidget` at 85-99 (`apps` is sorted by name, `pinned` is an `Option<Vec<String>>` of ids, `size` caches the last draw size); `shown()` at 121-124; `draw_tile` at 135-162 (badges at 148-153); `tiles` at 164-172; `layout` at 182-196; `max_scroll` at 198-200; `update` at 212-231 (the list arrives once as `Loaded::List`, 218-224); `draw` at 237-269 ("Loading apps…" at 240, scrollbar at 259-268); `click` at 271-309; `press` at 313-325; `release` at 349-355 (launches via `apps::launch`, 353); `wheel` at 357-363; tests at 378-409.
- `src/apps.rs`: `AppEntry { id, name }` at 32-37; `launch(id)` at 160-166 runs `ShellExecuteW` on a short-lived thread.

## Goal

Make the search bar useful. Add search hooks to the `Widget` trait, and implement them in the Apps widget. An empty query lists the installed pinned apps in pinned order (P1-D5). Typing ranks installed apps by the P1-D6 tiers and ties. Results show as the familiar tiles, 8 per row and 2 rows visible, with the highlighted one drawn with a stronger fill. Down/Up and Tab/Shift+Tab move the highlight in reading order and the grid scrolls to follow it; the wheel scrolls. Enter or a click launches the app and closes search without giving focus back, so the launched app comes to the front (P1-D8). The shell routes keys, clicks, wheel and drawing to the first widget that offers results; in Phase 1 that is always Apps.

## Exact Scope

Edit:

- `src/widgets/mod.rs` — `SearchKey`, `SearchAction`, five defaulted trait hooks.
- `src/widgets/apps.rs` — search state, `rank`, `results`, `follow`, result layout and drawing (sharing tile drawing and the scrollbar with the existing grid), the hook implementations, unit tests.
- `src/app.rs` — routing only: `App.results`, `query_changed`, key/click/wheel/draw routing for search, clearing results in `end_typing`.

Nothing else. No changes to `src/text.rs`, `src/apps.rs`, `src/clipboard.rs` or `src/gfx.rs`.

## Non-Goals

- No shortcut, `RegisterHotKey` or Settings row (Task 1-3).
- No search in other widgets (Clipboard, Media); the hooks default to "no results".
- No widget-hosted text fields or general keyboard hooks for widgets (P1-D12).
- No change to the Apps tab's own grid, picker, unpin badges or drag-to-reorder. Search results don't pin, unpin or reorder.
- No fuzzy matching beyond the four tiers; no rescanning of newly installed apps; no launch history or frequency ranking.
- Do not revert or commit unrelated changes. `docs/` is untracked (the roadmap and these Phase 1 docs): leave it alone and out of this task's code commit unless the user asks.

## Dependencies

Task 1-1, finished and verified. Task 1-3 relies on `query_changed` and on `open_search` showing the pinned apps.

## Step-by-step Work

1. `src/widgets/mod.rs`: add `SearchKey` and `SearchAction` after `Compact`, and the five hooks with defaults at the end of the trait.
2. `src/widgets/apps.rs`: write `rank`, `results` and `follow` with their tests first.
3. Add `Search` state to `AppsWidget`. Factor the tile's icon-and-label drawing and the scrollbar into helpers shared with `draw`, and add a cell helper shared with `layout`.
4. Implement `search`, `draw_search`, `search_key`, `click_search` and `wheel_search`, and recompute results in `update` when the app list arrives.
5. `src/app.rs`: add `results`, `query_changed`; call it when search opens and whenever the field's text changes; route Down/Up/Tab and Enter, content clicks, the wheel and drawing; act on `SearchAction::Done` with `end_typing(false)`.
6. Run the verification below. If the session ends here, relaunch the release exe.

## Test Expectations

New unit tests in `src/widgets/apps.rs`:

- `rank_tiers_follow_the_name_words_and_initials`
- `rank_is_case_insensitive_and_handles_non_ascii`
- `blank_query_lists_installed_pins_in_order`
- `results_order_by_tier_then_pinned_then_length_then_name`
- `follow_keeps_the_highlight_on_screen`

Expected total: 53 + 5 = 58 passing. The two existing `reorder` tests must still pass unchanged.

## Verification Commands

From the repo root, in PowerShell:

```powershell
cargo build
cargo clippy --all-targets
cargo test
```

Runtime checks follow ImplementationSpec-1-2 § Verification, with the same setup and cleanup as Task 1-1: quit the release instance, back up settings.ini and the clipboard text, run `target\debug\mynotch.exe`, use the "MyNotch focus test" window, then restore everything and relaunch the release exe if the session ends here.

## Acceptance Criteria

- `cargo clippy --all-targets` prints no warnings; `cargo test` reports 58 passed, 0 failed.
- Opening search shows the installed pinned apps in the same order as the Apps tab, with no × or ✓ badges and the first one highlighted.
- Typing filters and orders results per P1-D6 (for example, an app's initials find it, and a pinned app wins a tie). A query with no matches shows `No apps match "<query>"`.
- Down/Up and Tab/Shift+Tab move the highlight one result at a time, stopping at the ends, and the grid scrolls to keep it visible. Left/Right move the caret, not the highlight. The wheel scrolls the results.
- Enter launches the highlighted app; search closes; the launched app, not the previous window, ends up in the foreground. Clicking a result does the same.
- Esc, a tab click and a click elsewhere still behave as in Task 1-1.
- The Apps tab (grid, badges, picker, reorder, launch on release) looks and works as before.
- Every app launched by the tests was closed afterwards (only processes the test started), settings.ini and the clipboard were restored, and a release MyNotch is running when the session ends.

## Review Checklist

- [ ] The trait hooks have defaults, so the other widgets compile unchanged; `all()` order is untouched.
- [ ] `rank` and `results` are pure and match P1-D6 exactly, including the tie order (pinned, then shorter, then alphabetical) and the blank-query case (installed pins in pinned order).
- [ ] `query_changed` calls `search()` on every widget and picks the first that returns true; it runs when search opens and after every text change, and not for caret-only moves.
- [ ] `SearchAction::Done` ends typing with `restore = false`; no other path launches without closing search.
- [ ] `end_typing` clears `results` together with `search`, before any focus call.
- [ ] The highlight index stays within the results; scroll never passes the results' maximum.
- [ ] Shared drawing helpers leave the existing Apps grid pixel-identical (compare before/after screenshots).
- [ ] Only the three files in scope changed; `docs/` untouched; committed only if the session's workflow calls for it.
