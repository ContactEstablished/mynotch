# MyNotch — Handoff (2026-10-14)

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

## Current state — Milestone 4 (branch `milestone-4`, uncommitted; `milestone-3` pushed)
Builds clean (`cargo build`, `cargo clippy --all-targets`: no warnings; `cargo test`: 25 passing). A release build is left running from `target\release\mynotch.exe`, so debug `cargo build` isn't blocked by a locked exe. Quit it from the menu before `cargo build --release` or `cargo run`; the single-instance guard makes a second copy exit at once.

**What it does.** The collapsed notch shows a one-line readout: a running focus timer ("Focus 24:13"), else the current track while media plays, else a paused timer, else CPU/RAM. Hovering opens it (720 × 240) with tabs **Home, Media, Focus, Apps, Clipboard, System** and a pin. Opening goes to Home if that's where you left it; otherwise to live activity (running timer or playing media); otherwise to the last tab you picked.
- **Home:** cards side by side, weighted by `card_weight()`: now playing (art, title, progress, transport), focus (time, progress, controls) and system (CPU/RAM bars, NET).
- **Media:** 160 px album art (decoded with WIC on the media thread, fetched once per track with a few retries), title/artist/app, a progress bar with elapsed/total (position extrapolated from the app's last timeline report), click-to-seek, previous/play-pause/next.
- **Focus:** Pomodoro: focus 15/25/50 min (chips, saved), 5 min short break, 15 min long break after every 4 sessions. A ring empties as the phase runs (blue for focus, teal for breaks), with session dots. When a phase ends, the notch pops open for 6 s and plays `Notification.Reminder`.
- **Apps:** pinned apps from Windows' full app list (`shell:AppsFolder`: desktop and Store apps) with their real icons; click to launch. "+" opens a scrollable picker of every installed app: click to pin or unpin, "Done" to close. First run pre-pins installed common apps (`apps::SUGGESTED`). Pins are saved as `apps=` in settings.
- **Clipboard:** the last 30 copies (text, links, images with thumbnails), newest first, in memory only; click to copy one back ("Copied"); the trash button clears. Copies marked private (`ExcludeClipboardContentFromMonitorProcessing`, `Clipboard Viewer Ignore`, `CanIncludeInClipboardHistory` = 0, as set by password managers) are skipped. Reading waits 150 ms after each change, so it doesn't collide with apps still publishing their data.
- **System:** CPU, RAM and NET with 60-second history graphs.
- **Tray icon:** drawn at runtime (`icon.rs`) at the DPI-correct size, white or near-black to match the taskbar theme, with a blue-violet notch. Redrawn on theme/DPI change and Explorer restart.
- **Look:** painted obsidian glass; controls are translucent white.
- **Menu** (right-click the notch or the tray icon): Pin open, Hide over fullscreen apps (default on), Start with Windows (default off; `HKCU\...\Run\MyNotch`), Quit.
- **Settings** in `%APPDATA%\MyNotch\settings.ini`: `pinned`, `hide_in_fullscreen`, `tab`, `focus_minutes`, `apps`. The mouse wheel scrolls the Apps and Clipboard lists.

**Verified** on the dev machine (5120-wide primary monitor, 100% scale), via scripts that post messages and take screenshots. The user's settings file (and, for clipboard tests, the clipboard) were saved first and restored afterwards.
- M4: Home, Media, Focus and System at the new size. Album art from a silent test session that publishes a cover. Progress/time from a real browser session (Brave) and the test session. Apps: suggestions saved and drawn with real icons; the picker opens and scrolls with the wheel; clicking pins/unpins (saved); clicking a pinned app launches it (File Explorer opened, then the test closed it). Clipboard: text, link and image captured; a private copy skipped; four test copies all succeeded while MyNotch listened; clicking copies back and moves the item to the top; clear empties the list.
- Fullscreen hiding also seen working with a real app (Brave fullscreen hid the notch).
- Earlier milestones: media transport against a test session; focus session end and pop-open; polish (single instance, settings restore, menu toggles, fullscreen hide/show); click routing at the real cursor; tray icon; display-change relayout.
- **Not re-run after the M4 resize:** the click-routing (cursor-moving) and polish regression scripts; their hard-coded coordinates are for the old 520 × 150 size and need updating.
- Not verified: **seeking's effect** (Windows accepted the seek, but the test player never reports a new position; try with Spotify/YouTube); hover highlights; Next/Previous against a real player; the light-taskbar icon; a real DPI change; an Explorer restart.

| File | Purpose |
|---|---|
| `Cargo.toml` | deps: `sysinfo` 0.39, `windows` 0.62 (features for tray, SMTC, `Storage_Streams` + `Win32_Graphics_Imaging` for art, registry, mutex, audio, clipboard: `Win32_System_DataExchange`/`Memory`/`Ole`), `windows-numerics` 0.3 |
| `src/main.rs` | `windows_subsystem = "windows"`, calls `app::run()` |
| `src/app.rs` | Window, animation, tabs (tab 0 = Home, widget `i` = tab `i + 1`), Home card layout, input (click, wheel), tray/menu, fullscreen fade, click-through; hosts the widgets |
| `src/gfx.rs` | `Canvas` (text, fills, circles, arcs, clip, hover, `glass()`, `image()`), `Image` (shared pixels) + `ImageCache` (D2D bitmaps, cleared on device loss), `Brushes`/`Ink`, `Fonts`/`Font`, `glyph`s, `Rect` |
| `src/imaging.rs` | WIC decode (album art) and a box-filter `from_bgra` for raw pixels (unit-tested) |
| `src/media.rs` | SMTC worker: polls every 500 ms; art, timeline, commands incl. seek; `app_name()` (unit-tested) |
| `src/apps.rs` | AppsFolder enumeration + icons on an STA thread; `launch()`; `suggested()` (unit-tested) |
| `src/clipboard.rs` | Message-only listener window, debounced capture, private-copy rules, copy-back; DIB decoder, `is_link`, `preview` (unit-tested) |
| `src/desktop.rs` | Single instance, autostart, fullscreen rule (unit-tested), taskbar theme |
| `src/settings.rs` | `Settings` key=value file; tolerant parser (unit-tested) |
| `src/timer.rs` | Pomodoro state machine (unit-tested) |
| `src/icon.rs` | Runtime tray icon |
| `src/stats.rs` | `Stats::sample()` -> `Snapshot { cpu, ram, down, up }` |
| `src/widgets/mod.rs` | `Widget` trait: `update`, `compact`, `draw`, `click`, `wheel`, `load`/`store`, `take_attention`, and `card_weight`/`draw_card`/`click_card` for Home. Shared `button`, `button_row`, `clock`. `all()` is the tab order |
| `src/widgets/{media,focus,apps,clipboard,system}.rs` | The panels |

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
- Update the regression scripts for the 720 × 240 layout and re-run them.
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
