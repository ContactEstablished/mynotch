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

## Current state — Milestone 1 done
Builds clean (`cargo build`, no warnings) and runs. The `msvcrt.lib` blocker was fixed by installing the VS C++ workload.

Verified on the dev machine (5120-wide primary monitor, 100% scale):
- Notch appears top-center, expands on hover, collapses on leave; text is crisp.
- Per-pixel click-through: transparent areas and the rounded corners pass clicks to the window underneath (checked with `WindowFromPoint`).
- Pin keeps it open; tray icon registers and is removed on exit; the Pin/Quit menu works; resolution changes re-layout correctly.
- Not yet tested for real: an actual DPI change (only the message path was exercised) and a real Explorer restart re-adding the tray icon.

| File | Purpose |
|---|---|
| `Cargo.toml` | deps: `sysinfo` 0.39, `windows` 0.62 (+ `Win32_UI_Shell` for the tray), `windows-numerics` 0.3 (`Vector2` is D2D's point type in windows 0.62) |
| `src/main.rs` | `windows_subsystem = "windows"`, calls `app::run()` |
| `src/stats.rs` | `Stats::sample()` -> `Snapshot { cpu, ram }` (percent) |
| `src/app.rs` | Window, rendering, animation, tray/menu (details below) |

### How `app.rs` works
- Layered window (`WS_EX_LAYERED | TOPMOST | TOOLWINDOW | NOACTIVATE`) the size of the *expanded* footprint. The notch shape is drawn inside it, and transparent pixels let clicks through. Position and size are applied by `UpdateLayeredWindow` on every render.
- `Layout` holds everything that depends on the primary monitor: DPI scale, window rect, the DIB `Surface` (frees its GDI objects on drop), and the DirectWrite text formats. It is rebuilt on `WM_DISPLAYCHANGE` / `WM_DPICHANGED`.
- Rendering: `ID2D1DCRenderTarget` (premultiplied BGRA) bound to the surface's memory DC. Brushes are cached in `Brushes`. On `D2DERR_RECREATE_TARGET`, the target and brushes are rebuilt.
- A 16 ms `WM_TIMER` polls the cursor, hit-tests against the animated notch rect (+6 px `HOVER_MARGIN`), and eases `t` (0 = collapsed, 1 = expanded) with smoothstep. A 1 s timer samples stats. Redraws only happen when `dirty` is set.
- Input: left-click the pin dot to toggle pinning. Right-click the notch, or right-click the tray icon, for the menu (Pin open / Quit). Left-click the tray icon to toggle pinning. The menu runs outside the `APP` borrow so animation continues while it's open. The tray icon is re-added on `TaskbarCreated`.
- `App` is dropped explicitly after the message loop, so COM objects aren't released during thread-local teardown at process exit.

## To resume
`cargo run`. To quit: right-click the notch or the tray icon -> Quit.

## Known rough edges / next steps
- Only the primary monitor is supported (by design for now).
- Stays topmost over fullscreen apps (games/video); should auto-hide when the foreground window is fullscreen.
- Opening the menu takes foreground from the active app (standard tray-menu behavior).
- Generic `IDI_APPLICATION` tray icon; needs a real icon resource.
- First CPU sample reads 0% (sysinfo needs two samples a refresh apart).
- Clippy isn't installed: `rustup component add clippy`.
- Network stats not yet implemented (CPU/RAM only).
- No commits yet. The repo is initialized on `master` with `.gitignore` (`/target`).
- Later milestones: media controls (Windows SMTC), focus timer + themes, tasks/notes, clipboard history, shortcuts, AI-app detection, GitHub activity, code playground. Add a widget trait/registry before the second widget.
