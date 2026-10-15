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

## Current state — Milestone 2 done (branch `milestone-2`)
Builds clean (`cargo build`, `cargo clippy --all-targets`: no warnings; `cargo test`: 1 passing).

The expanded notch has a tab row (Media, System) and a pin glyph. The collapsed notch shows the highest-priority widget's one-line readout: the current track while media is playing, otherwise CPU/RAM. Expanding from collapsed opens the widget whose readout was showing.

Verified on the dev machine (5120-wide primary monitor, 100% scale), using a silent test media session (Windows PowerShell 5.1 + `Windows.Media.Playback.MediaPlayer`):
- Collapsed readout shows "♪ title — artist", ellipsis-trimmed; it falls back to CPU/RAM when nothing plays.
- Media tab: title, artist, source app; disabled buttons are greyed out; play/pause and resume work against the real session; "Nothing playing" appears when the session ends.
- Tab switching, auto-selected tab, and the pinned/unpinned glyphs all work. Idle cost is about 0.6% of one core (debug build).
- Re-checked after the Milestone 2 changes: click-through on the collapsed and expanded shapes (including rounded corners), tray icon register/remove, pin/unpin via the tray (collapses again when unpinned), display-change relayout, and menu Quit.
- Not tested: hover-to-expand and button/tab hover highlights (they need real cursor movement), Next/Previous against a real player, a real DPI change, and an Explorer restart.

| File | Purpose |
|---|---|
| `Cargo.toml` | deps: `sysinfo` 0.39, `windows` 0.62 (+ `Win32_UI_Shell` for the tray, `Media_Control` for SMTC), `windows-numerics` 0.3 (`Vector2` is D2D's point type in windows 0.62) |
| `src/main.rs` | `windows_subsystem = "windows"`, calls `app::run()` |
| `src/app.rs` | Window, animation, tab row, input, tray/menu; hosts the widgets |
| `src/gfx.rs` | `Canvas` (logical-unit drawing: text, fills, circles, clip, hover test), `Brushes`/`Ink`, `Fonts`/`Font`, icon `glyph`s, `Rect` |
| `src/widgets/mod.rs` | `Widget` trait, `Compact` readout, `all()` registry (tab order) |
| `src/widgets/media.rs` | Now-playing panel and transport buttons |
| `src/widgets/system.rs` | CPU/RAM bars (samples every 1 s; shows "–" until the first valid sample) |
| `src/media.rs` | SMTC worker thread: polls every 500 ms, runs commands; talks to the UI over channels. `app_name()` turns an AUMID into a friendly name (unit-tested) |
| `src/stats.rs` | `Stats::sample()` -> `Snapshot { cpu, ram }` (percent) |

### How it works
- Layered window (`WS_EX_LAYERED | TOPMOST | TOOLWINDOW | NOACTIVATE`) the size of the *expanded* footprint. The notch shape is drawn inside it, and transparent pixels let clicks through. Position and size are applied by `UpdateLayeredWindow` on every render.
- `Layout` holds everything that depends on the primary monitor: DPI scale, window rect, the DIB `Surface`, and `Fonts`. It is rebuilt on `WM_DISPLAYCHANGE` / `WM_DPICHANGED`.
- Rendering: `ID2D1DCRenderTarget` bound to the surface's memory DC. Cached `Brushes` are rebuilt with the target on `D2DERR_RECREATE_TARGET`.
- The expanded layout is fixed-size (logical constants `PAD`, `tab_rect`, `PIN`, `CONTENT` in `app.rs`). It is centred and clipped to the animating body, so content never reflows mid-animation.
- **Widgets** draw in logical units relative to their content area through `Canvas`. `update(now)` runs every 16 ms frame (widgets throttle themselves) and returns whether a redraw is needed. `click(x, y, w, h)` gets content-relative logical coordinates. To add a widget: implement `Widget` and add it to `widgets::all()`.
- Media: all WinRT calls stay on the `media` worker thread (MTA), so a hung player can't stall the UI. After a command the worker always re-reports state, which corrects the UI's optimistic play/pause toggle if the command was ignored.
- Input: click a tab to switch; click the pin glyph to pin. Right-click the notch or the tray icon for the menu (Pin open / Quit). Left-click the tray icon to toggle pinning.
- `App` is dropped explicitly after the message loop, so COM objects aren't released during thread-local teardown.

## To resume
`cargo run`. To quit: right-click the notch or the tray icon -> Quit.

## Known rough edges / next steps
- Media: no album art yet (session `Thumbnail()` needs `Storage_Streams` plus WIC decoding into a D2D bitmap); no progress bar (timeline position must be extrapolated from `LastUpdatedTime`); it polls instead of subscribing to SMTC change events.
- Only the primary monitor is supported (by design for now).
- Stays topmost over fullscreen apps (games/video); should auto-hide when the foreground window is fullscreen.
- Opening the menu takes foreground from the active app (standard tray-menu behavior).
- Generic `IDI_APPLICATION` tray icon; needs a real icon resource.
- Network stats not yet implemented (CPU/RAM only).
- Repo: `origin` = github.com/ContactEstablished/mynotch, default branch `main`.
- Later milestones: focus timer + themes, tasks/notes, clipboard history, shortcuts, AI-app detection, GitHub activity, code playground.
