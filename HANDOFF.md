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

## Current state — Milestone 3 (branch `milestone-3`, uncommitted; `milestone-2` holds M2 + polish, pushed)
Builds clean (`cargo build`, `cargo clippy --all-targets`: no warnings; `cargo test`: 7 passing). A release build is left running from `target\release\mynotch.exe`, so normal debug `cargo build`/`cargo run` aren't blocked by a locked exe. Quit it from the menu before `cargo build --release`.

**What it does.** The collapsed notch shows a one-line readout: a running focus timer ("Focus 24:13"), else the current track while media plays, else a paused timer, else CPU/RAM. Hovering opens it with tabs (Media, Focus, System) and a pin. Opening goes to the live activity (running timer or playing media), otherwise to the last tab you picked.
- **Media:** title, artist, source app, previous/play-pause/next (via SMTC).
- **Focus:** Pomodoro timer: focus 15/25/50 min (chips, saved), 5 min short break, 15 min long break after every 4 sessions. A ring empties as the phase runs (blue for focus, teal for breaks), with dots for the sessions done this cycle; there are reset, start/pause and skip buttons. When a phase ends, the notch pops open on Focus for 6 s (`PEEK`) and plays the `Notification.Reminder` sound. The next phase waits for you to press play.
- **System:** CPU and RAM bars, plus NET (↓/↑ throughput).
- **Tray icon:** drawn at runtime (`icon.rs`) at the DPI-correct small-icon size: a screen outline (white on a dark taskbar, near-black on a light one) with a blue-violet notch. It is redrawn on theme change (`WM_SETTINGCHANGE` "ImmersiveColorSet"), DPI change and Explorer restart, and also set as the window icon.
- **Look:** painted obsidian glass (near-black see-through gradient, upper-left reflection, violet sheen, bottom rim light, shadow when open); controls are translucent white.
- **Menu** (right-click the notch or the tray icon): Pin open, Hide over fullscreen apps (default on), Start with Windows (default off; writes `HKCU\...\Run\MyNotch` = the running exe's path), Quit.
- **Settings** in `%APPDATA%\MyNotch\settings.ini` (`pinned`, `hide_in_fullscreen`, `tab`, `focus_minutes`). Start with Windows lives only in the registry.
- **One instance only** (`Local\MyNotch.SingleInstance` mutex); a second launch exits silently.

**Verified** on the dev machine (5120-wide primary monitor, 100% scale), via scripts that post messages to the window and take screenshots:
- Media with a silent test session (Windows PowerShell 5.1 + `MediaPlayer`): readout, panel, greyed-out buttons, pause/resume, "Nothing playing".
- Click routing at the real cursor position: the body takes clicks; shadow, rounded corners, and everything when the cursor is elsewhere (even pinned) pass through. Hover-to-expand works.
- Polish: second instance exits; pin and picked tab survive a restart; the menu toggles work (Start with Windows adds and removes the Run value; the fullscreen toggle is saved); a borderless window covering the monitor hides the notch (fade out, then `SW_HIDE`); a maximized window doesn't; it comes back when the fullscreen window closes; with hiding off it stays.
- Tray icon register/remove, display-change relayout, menu Quit. Idle cost about 0.6% of one core (debug).
- Milestone 3 (scripted, saving and restoring the real settings file): Focus tab idle/running/break visuals; chips save `focus_minutes`; the collapsed readout shows the running timer above playing media; a real 1-minute session ended and popped the notch open on Focus showing the break. Tray icon registered and set as the window icon; its 16 px pixels checked (opaque, crisp outline, clear inside) on a dark taskbar.
- Not tested: whether the end-of-session sound is audible, the light-taskbar icon variant, live theme switching, button/tab hover highlights, Next/Previous against a real player, a real DPI change, an Explorer restart, a real game in exclusive fullscreen.

| File | Purpose |
|---|---|
| `Cargo.toml` | deps: `sysinfo` 0.39, `windows` 0.62 (+ `Win32_UI_Shell` tray, `Media_Control` SMTC, `Win32_System_Registry` autostart, `Win32_System_Threading`/`Win32_Security` mutex), `windows-numerics` 0.3 (`Vector2` is D2D's point type in windows 0.62) |
| `src/main.rs` | `windows_subsystem = "windows"`, calls `app::run()` |
| `src/app.rs` | Window, animation, tab row, input, tray/menu, fullscreen fade, click-through toggle; hosts the widgets |
| `src/gfx.rs` | `Canvas` (logical-unit drawing: text, fills, circles, clip, hover test, `glass()` for the obsidian body), `Brushes`/`Ink` (+ `Glass` gradient brushes), `Fonts`/`Font`, icon `glyph`s, `Rect` |
| `src/desktop.rs` | Single-instance guard, `autostart` (Run key), `foreground_is_fullscreen` (unit-tested rule: covers the monitor, not maximized, not the desktop) |
| `src/settings.rs` | `Settings` load/save as `key=value` lines; tolerant parser (unit-tested) |
| `src/timer.rs` | Pomodoro state machine (`Timer`, `Phase`); time is passed in (unit-tested) |
| `src/icon.rs` | Draws the tray icon with D2D into a DIB, un-premultiplies the alpha, `CreateIconIndirect` |
| `src/widgets/focus.rs` | Focus timer panel: ring, digits, label, chips, controls |
| `src/widgets/mod.rs` | `Widget` trait (+ `load`/`store` for settings, `take_attention` to pop open), `Compact` readout (`priority`, `live`), `all()` registry (tab order) |
| `src/widgets/media.rs` | Now-playing panel and transport buttons |
| `src/widgets/system.rs` | CPU/RAM bars and NET throughput (samples every 1 s; "–" until the first valid sample); `rate()` formatter (unit-tested) |
| `src/media.rs` | SMTC worker thread: polls every 500 ms, runs commands, talks to the UI over channels. `app_name()` turns an AUMID into a friendly name (unit-tested) |
| `src/stats.rs` | `Stats::sample()` -> `Snapshot { cpu, ram, down, up }` |

### How it works
- Layered window (`WS_EX_LAYERED | TOPMOST | TOOLWINDOW | NOACTIVATE`) the size of the *expanded* footprint plus `SHADOW` (18 px) of room on the sides and bottom. Position, size and fade alpha are applied by `UpdateLayeredWindow` on every render.
- Click-through: the frame timer toggles `WS_EX_TRANSPARENT` so the window takes clicks only while the cursor is inside the body shape (`in_notch`: rect with rounded bottom corners). `WindowFromPoint` checks are only meaningful at the cursor position.
- Fullscreen: every 500 ms (`FULLSCREEN_POLL`) the tick checks the foreground window. When hiding, `fade` eases to 0 (window `SW_HIDE`, no drawing, no hover); afterwards it shows and fades back in.
- `Layout` holds everything that depends on the primary monitor: DPI scale, window rect, monitor rect, the DIB `Surface`, and `Fonts`. It is rebuilt on `WM_DISPLAYCHANGE` / `WM_DPICHANGED`.
- Rendering: `ID2D1DCRenderTarget` bound to the surface's memory DC. Cached `Brushes` are rebuilt with the target on `D2DERR_RECREATE_TARGET`.
- The expanded layout is fixed-size (logical constants `PAD`, `tab_rect`, `PIN`, `CONTENT` in `app.rs`). It is centred and clipped to the animating body, so content never reflows mid-animation.
- **Widgets** draw in logical units relative to their content area through `Canvas`. `update(now)` runs every 16 ms frame (widgets throttle themselves) and returns whether a redraw is needed. `click(x, y, w, h)` gets content-relative logical coordinates. To add a widget: implement `Widget` and add it to `widgets::all()`.
- Media: all WinRT calls stay on the `media` worker thread (MTA), so a hung player can't stall the UI. After a command the worker always re-reports state, which corrects the optimistic play/pause toggle.
- `App` is dropped explicitly after the message loop, so COM objects aren't released during thread-local teardown.

## To resume
`cargo run` (quit the running release instance first, or the new one exits at once because of the single-instance guard). To quit: right-click the notch or the tray icon -> Quit.

## Known rough edges / next steps
- Start with Windows registers whichever exe was running when it was ticked. A proper install location would make that stable.
- The glass is painted, not a real backdrop blur. True acrylic/blur behind a custom-shaped, animated window would mean moving rendering from `UpdateLayeredWindow` to DirectComposition.
- Media: no album art yet (session `Thumbnail()` needs `Storage_Streams` plus WIC decoding into a D2D bitmap); no progress bar (timeline position must be extrapolated from `LastUpdatedTime`); it polls instead of subscribing to SMTC change events.
- Network throughput sums all non-loopback interfaces, so traffic through virtual adapters (VPN, WSL/Hyper-V) can be counted twice.
- Only the primary monitor is supported (by design for now). Fullscreen apps on other monitors don't hide the notch.
- Opening the menu takes foreground from the active app (standard tray-menu behavior).
- The exe itself still has the default icon in Explorer/Task Manager (embedding one needs a resource compiler step, e.g. `embed-resource` in `build.rs`).
- Focus timer: the 7 visual themes BuildNotch offers (Clean, Pixel, Dusk, Dots, Retro Clock, Terminal, Cyber HUD) aren't built yet; this is the "Clean" look. Breaks don't auto-start, and a running timer doesn't survive a restart.
- Repo: `origin` = github.com/ContactEstablished/mynotch, default branch `main`.
- Next features: timer themes, album art/progress, app shortcuts, AI-app detection + usage, clipboard history, GitHub activity, notes/tasks/code playground (these need text input inside the notch).
