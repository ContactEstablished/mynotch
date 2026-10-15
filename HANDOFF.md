# MyNotch — Handoff (2026-10-15)

## Goal
Write our own version of BuildNotch (https://www.buildnotch.app/), a Windows 10/11 "notch" overlay that sits at the top of the screen, expands on hover (or when pinned), and hosts developer-oriented widgets.

BuildNotch's feature set (reference for future milestones):
- Music/media playback controls
- Focus timers (7 visual themes: Clean, Pixel, Dusk, Dots, Retro Clock, Terminal, Cyber HUD)
- Tasks and quick notes per project
- AI app detection/launching (ChatGPT, Claude, Cursor, Antigravity) and daily/weekly AI usage tracking
- GitHub contribution activity
- Code snippet playground (auto-saves locally)
- Clipboard history (text, code, images, links)
- App shortcuts, inspirational quotes
- Live CPU / RAM / network stats

## Decisions made
- **Language/UI:** native Rust + `windows` crate (0.62) + Direct2D/DirectWrite. No web view, no GUI framework.
- **Milestone 1 scope:** overlay shell + live system stats (CPU/RAM). Proves the hard part (transparent, always-on-top, hover-animated window) first.
- **Milestone 2 scope:** widget system + media controls (SMTC). The System stats moved onto the widget system.
- **Polish pass (after M2):** obsidian glass look, fullscreen auto-hide, Start with Windows, saved settings, network speed, single instance. Done before adding more widgets.
- **Milestone 3:** Pomodoro focus timer widget + a runtime-drawn, theme-aware tray icon.
- **Milestone 4:** bigger notch (720 × 240) with a Home dashboard + tabs; album art, progress and seeking in Media; app shortcuts; clipboard history. (The user's picks.)
- **After M4 (user requests):** unpin apps straight from the Apps grid; a Clock tab with seconds; icon tabs drawn as outlined tabs; slide the notch along the top edge; a Settings tab (gear) for body colour and opacity.
- **Second round:** Clock and Focus merged into one **Time** tab (clock left, focus middle, a new stopwatch right); a 5-minute-step slider for the focus length beside the 15/25/50 chips; drag pinned apps to reorder them.

## Current state — after Milestone 4 (branch `milestone-4`, pushed: the M4 commit `9dd152c`, then the post-M4 polish on top)
Builds clean (`cargo build`, `cargo clippy --all-targets`: no warnings; `cargo test`: 39 passing). A release build is left running from `target\release\mynotch.exe`, so debug `cargo build` isn't blocked by a locked exe. Quit it from the menu before `cargo build --release` or `cargo run`; the single-instance guard makes a second copy exit at once.

**What it does.** The collapsed notch shows a one-line readout: a running focus timer ("Focus 24:13"), else the current track while media plays, else a paused timer, else CPU/RAM. Hovering opens it (720 × 240). The tab row has icon tabs **Home, Media, Time, Apps, Clipboard, System** on the left and **Settings** (gear) on the right, then the pin. Tabs are outlined, with rounded tops, standing on a line that opens under the active tab, so it joins the panel below. The empty stretch between the tabs shows the hovered (else active) tab's name and a six-dot grip. Opening goes to Home if that's where you left it; otherwise to live activity (running timer or playing media); otherwise to the last tab you picked. Settings is never reopened by itself.
- **Sliding:** drag the empty stretch of the tab row (↔ cursor there). The position is saved as a fraction of the monitor width (`position`, 0 = collapsed notch flush left, 1 = flush right, 0.5 = centre), so it survives resolution changes. Collapsed and expanded centre on the same point, except that the expanded notch keeps its shadow on-screen (`edges()`), so near an edge it stops while the collapsed pill carries on to the edge. A drag that ends within 12 px of the middle snaps to it. The notch is never the foreground window, so mouse capture only sees the cursor over the window; the frame timer also follows the drag and ends it when the physical button is up (`primary_button_down`, which honours swapped buttons).
- **Settings:** nine colour presets (Obsidian default, Graphite, Slate, Navy, Ocean, Forest, Plum, Wine, Ember) and a custom swatch that opens Windows' colour picker (`ChooseColorW` with a hook). The picker opens centred under the notch, previews each colour on the notch live (from the dialog's R/G/B boxes' `EN_CHANGE`), and Cancel goes back. Its custom-colour slots start as the presets. Opacity is a slider from 10–100% in 5% steps (the wheel works over it too). It scales the body, sheen, glow and shadow, not the content or the rim. A Center button resets the position. The glass is generated from one colour: top = lighten 3%, bottom = ×0.4, glow = the hue at S ≥ 0.6, V = 0.9. The default `0B0912` reproduces the original obsidian look (unit-tested).
- **Time tab:** three cards in Home's style, sized 30/40/30% (`time::cards`). All three share a header line, then a big number on one line.
  - **Clock:** `h:mm:ss` with a smaller AM/PM, or `HH:mm:ss` if the user's regional setting is 24-hour (`LOCALE_ITIME`). Below it, the weekday (`dddd`) and month-day, using the locale's `LOCALE_SMONTHDAY` picture (`DATE_MONTHDAY` fails with error 87 on this machine). It always shows seconds, whatever the user's long time format says.
  - **Focus:** see below.
  - **Stopwatch:** `m:ss.t` (`h:mm:ss` past an hour), with start/pause and reset (reset is disabled at zero). In the collapsed notch: running, priority 8 and live; paused, priority 4.
- **Home:** cards side by side, weighted by `card_weight()`: now playing (art, title, progress, transport), focus (time, progress, controls) and system (CPU/RAM bars, NET).
- **Media:** 160 px album art (decoded with WIC on the media thread, fetched once per track with a few retries), title/artist/app, a progress bar with elapsed/total (position extrapolated from the app's last timeline report), click-to-seek, previous/play-pause/next.
- **Focus** (the Time tab's middle card, and a Home card): Pomodoro with a 5 min short break, and a 15 min long break after every 4 sessions.
  - The card shows the phase label, the time with reset/play/skip beside it, and a progress bar (blue for focus, teal for breaks).
  - The focus length is set with chips (15/25/50) or a slider from 5 to 180 min in 5 min steps (`minutes_at`, unit-tested), and saved as `focus_minutes`.
  - Chips and slider work only while a focus session waits to start; otherwise they're greyed out.
  - When a phase ends, the notch pops open for 6 s and plays `Notification.Reminder`.
- **Apps:** pinned apps from Windows' full app list (`shell:AppsFolder`: desktop and Store apps) with their real icons.
  - Click to launch; it launches on release.
  - Drag a pinned app to reorder: past 6 px it lifts and follows the cursor, the others shift live, and the order is saved on release (`reorder`, unit-tested; pinned-but-uninstalled apps keep their places).
  - Hovering a pinned app shows a × badge (red under the cursor) that unpins it. "+" opens a scrollable picker of every installed app (pinned ones carry a blue ✓): click to pin or unpin, "Done" to close. First run pre-pins installed common apps (`apps::SUGGESTED`). Pins are saved as `apps=` in settings.
- **Clipboard:** the last 30 copies (text, links, images with thumbnails), newest first, in memory only; click to copy one back ("Copied"); the trash button clears. Copies marked private (`ExcludeClipboardContentFromMonitorProcessing`, `Clipboard Viewer Ignore`, `CanIncludeInClipboardHistory` = 0, as set by password managers) are skipped. Reading waits 150 ms after each change, so it doesn't collide with apps still publishing their data.
- **System:** CPU, RAM and NET with 60-second history graphs.
- **Tray icon:** drawn at runtime (`icon.rs`) at the DPI-correct size, white or near-black to match the taskbar theme, with a blue-violet notch. Redrawn on theme/DPI change and Explorer restart.
- **Look:** painted obsidian glass; controls are translucent white.
- **Menu** (right-click the notch or the tray icon): Pin open, Hide over fullscreen apps (default on), Start with Windows (default off; `HKCU\...\Run\MyNotch`), Quit.
- **Settings** in `%APPDATA%\MyNotch\settings.ini`: `pinned`, `hide_in_fullscreen`, `tab`, `focus_minutes`, `apps`, `color` (`RRGGBB`), `opacity` (percent), `position` (0..1). The mouse wheel scrolls the Apps and Clipboard lists.

**Verified** on the dev machine (5120-wide primary monitor, 100% scale), via scripts that post messages and take screenshots. The user's settings file (and, for clipboard tests, the clipboard) were saved first and restored afterwards.
- Post-M4: every tab's icon and the outlined-tab look (Media, Clock, Apps, Settings, Home); the Settings tab opens and isn't remembered. Presets (Navy, Wine) recolour the notch and are saved; the opacity slider sets and saves (60%, 100%). The picker opens centred under the notch, previews live, Cancel restores, OK saves (`145A50`). Saved positions 0, 0.1 and 1 place the window as computed: collapsed pill flush left, expanded notch 18 px in. Real-mouse drags (SendInput, cursor restored after) slide by exactly the distance dragged, both ways. A release off the window ends the drag, and later moves don't drag. A drag that ends 10 px from centre snaps to it. ↔ cursor over the grip, arrow over tabs. Hovering an app shows the × (red when hovered); clicking it unpins and saves. The picker shows ✓ on pinned apps.
- Second round: the Time tab's three cards; the slider click sets and saves (35), the chip sets 25, and a real-mouse slider drag sets 70. While a session runs, the chips and slider grey out and a slider press changes nothing; reset restores them. Stopwatch start, run and pause. A real-mouse drag moved File Explorer from slot 0 to slot 2: it lifts mid-drag, the others shift, the order is saved. A plain click still launches (one File Explorer window opened, then the test closed it) and leaves the order alone.
- Not verified post-M4: a 24-hour locale's clock; the picker's keyboard focus after a real (not posted) click; a non-100% DPI with the new tab row; a monitor change while the notch is off-centre.
- M4: Home, Media, Focus and System at the new size. Album art from a silent test session that publishes a cover. Progress/time from a real browser session (Brave) and the test session. Apps: suggestions saved and drawn with real icons; the picker opens and scrolls with the wheel; clicking pins/unpins (saved); clicking a pinned app launches it (File Explorer opened, then the test closed it). Clipboard: text, link and image captured; a private copy skipped; four test copies all succeeded while MyNotch listened; clicking copies back and moves the item to the top; clear empties the list.
- Fullscreen hiding also seen working with a real app (Brave fullscreen hid the notch).
- Earlier milestones: media transport against a test session; focus session end and pop-open; polish (single instance, settings restore, menu toggles, fullscreen hide/show); click routing at the real cursor; tray icon; display-change relayout.
- **Not re-run after the M4 resize:** the click-routing (cursor-moving) and polish regression scripts; their hard-coded coordinates are for the old 520 × 150 size and need updating.
- Not verified: **seeking's effect** (Windows accepted the seek, but the test player never reports a new position; try with Spotify/YouTube); hover highlights; Next/Previous against a real player; the light-taskbar icon; a real DPI change; an Explorer restart.

| File | Purpose |
|---|---|
| `Cargo.toml` | deps: `sysinfo` 0.39, `windows` 0.62 (features for tray, SMTC, `Storage_Streams` + `Win32_Graphics_Imaging` for art, registry, mutex, audio, clipboard: `Win32_System_DataExchange`/`Memory`/`Ole`; clock: `Win32_Globalization` + `Win32_System_SystemInformation`; colour picker: `Win32_UI_Controls_Dialogs`; drag capture: `Win32_UI_Input_KeyboardAndMouse`), `windows-numerics` 0.3 |
| `src/main.rs` | `windows_subsystem = "windows"`, calls `app::run()` |
| `src/app.rs` | Window, animation, placement along the top edge (`edges`, `Layout::place`, unit-tested), tabs (tab 0 = Home, widget `i` = tab `i + 1`, Settings = `widgets.len() + 1`), the tab row (`draw_tabs`), Home card layout, input (press, drag via `Drag`, wheel), the colour picker (`pick_color`, run outside the `APP` borrow like the menu), tray/menu, fullscreen fade, click-through; hosts the widgets |
| `src/gfx.rs` | `Canvas` (text, `measure`, fills, `line`, `tab`, circles, clip, hover, `glass()`, `image()`), `Image` (shared pixels) + `ImageCache` (D2D bitmaps, cleared on device loss), `Brushes` (glass rebuilt by `restyle`)/`Ink` (incl. `Line`, `Edge`, any `Rgb`), `Rgb` + `Look` (body colour/opacity; unit-tested), `Fonts`/`Font`, `glyph`s, `Rect` |
| `src/settings_panel.rs` | The Settings tab: presets, custom swatch, opacity slider, Center; `pick_color` (hooked `ChooseColorW`: placement + live preview); hit-testing and slider maths (unit-tested) |
| `src/imaging.rs` | WIC decode (album art) and a box-filter `from_bgra` for raw pixels (unit-tested) |
| `src/media.rs` | SMTC worker: polls every 500 ms; art, timeline, commands incl. seek; `app_name()` (unit-tested) |
| `src/apps.rs` | AppsFolder enumeration + icons on an STA thread; `launch()`; `suggested()` (unit-tested) |
| `src/clipboard.rs` | Message-only listener window, debounced capture, private-copy rules, copy-back; DIB decoder, `is_link`, `preview` (unit-tested) |
| `src/desktop.rs` | Single instance, autostart, fullscreen rule (unit-tested), taskbar theme |
| `src/settings.rs` | `Settings` key=value file; tolerant parser (unit-tested) |
| `src/timer.rs` | Pomodoro state machine (unit-tested) |
| `src/icon.rs` | Runtime tray icon |
| `src/stats.rs` | `Stats::sample()` -> `Snapshot { cpu, ram, down, up }` |
| `src/widgets/mod.rs` | `Widget` trait: `title`, `icon`, `update`, `compact`, `draw`, `click`, `press`/`drag`/`release` (a widget-owned drag; the shell holds the mouse as `Drag::Widget`), `wheel`, `load`/`store`, `take_attention`, and `card_weight`/`draw_card`/`click_card` for Home. Shared `button`, `button_row`, `clock`, `slider`/`slider_at`/`slider_hit` (also used by Settings). `all()` is the tab order |
| `src/widgets/time.rs` | The Time tab: lays out and routes to its three cards, `clock.rs` (`Clock`), `focus.rs` (`Focus`, also Home's card) and `stopwatch.rs` (`Stopwatch`, unit-tested) |
| `src/widgets/{media,apps,clipboard,system}.rs` | The other panels |

### How it works
- Layered window (`WS_EX_LAYERED | TOPMOST | TOOLWINDOW | NOACTIVATE`) sized to the expanded footprint plus `SHADOW` (18 px). Position, size and fade alpha are applied by `UpdateLayeredWindow` on every render.
- Click-through: the frame timer toggles `WS_EX_TRANSPARENT` so the window takes clicks only while the cursor is inside the body shape (`in_notch`).
- Fullscreen: every 500 ms the foreground window is checked; the notch fades out and hides, then returns.
- Rendering: `ID2D1DCRenderTarget` on a DIB; `Brushes` and `ImageCache` are rebuilt or cleared on `D2DERR_RECREATE_TARGET`. Images cross threads as `Arc<Image>` (premultiplied BGRA) and become D2D bitmaps lazily.
- Workers: media (MTA, WinRT), apps (STA, shell) and launches (short-lived STA threads) never block the UI. Clipboard capture runs on the UI thread through its own message-only window.
- Widgets work in logical units; the shell persists widget preferences (`store`) after clicks and updates.
- `App` is dropped explicitly after the message loop.

## To resume
Quit the running release instance (right-click the notch or the tray icon -> Quit), then `cargo run`.

## Known rough edges / next steps
- Update the regression scripts for the 720 × 240 layout and re-run them. (The tab row changed again post-M4: 44 × 32 tabs from x = 24, pitch 48, six of them (Time at index 2, Apps at 3); Settings tab at x = 620.)
- Apps: dragging doesn't scroll the grid, so an app can only be dropped on the visible rows.
- Look: light custom colours make the white content hard to read; text doesn't switch to dark.
- Sliding: near an edge the expanded notch stops while the pill's position keeps moving, which you only see once it collapses.
- Apps: no search in the picker (needs text input); the app list loads once per launch, so newly installed apps appear after a restart.
- Clipboard: history is lost on restart (by design, for privacy); only text and bitmaps are kept (no files or rich formats); very large images are converted on the UI thread.
- Media: seeking's effect is unverified; it still polls instead of subscribing to SMTC events.
- Start with Windows registers whichever exe was running when it was ticked; a proper install location would make that stable.
- The glass is painted, not a real backdrop blur (that would mean moving to DirectComposition).
- Network throughput can double-count virtual adapters (VPN, WSL).
- Primary monitor only. The exe itself has no embedded icon.
- Focus timer: other visual themes, auto-start breaks, surviving restarts.
- Repo: `origin` = github.com/ContactEstablished/mynotch, default branch `main`.
- Next features: AI-app tracker (detect/launch + daily usage), timer themes, GitHub activity, notes/tasks/code playground (these need text input inside the notch).
