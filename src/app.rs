use std::cell::RefCell;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use windows::{
    Win32::{
        Foundation::*,
        Graphics::{
            Direct2D::{Common::*, *},
            DirectWrite::*,
            Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM,
            Gdi::*,
        },
        Media::Audio::{PlaySoundW, SND_ALIAS, SND_ASYNC},
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            HiDpi::*,
            Input::KeyboardAndMouse::{GetAsyncKeyState, ReleaseCapture, SetCapture, VK_LBUTTON, VK_RBUTTON},
            Shell::{
                NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFY_ICON_MESSAGE, NOTIFYICONDATAW,
                Shell_NotifyIconW,
            },
            WindowsAndMessaging::*,
        },
    },
    core::*,
};

use crate::desktop::{self, autostart};
use crate::gfx::{Align, Brushes, Canvas, Font, Fonts, ImageCache, Ink, Look, Rect, glyph};
use crate::icon;
use crate::settings::Settings;
use crate::settings_panel::{self, Hit};
use crate::widgets::{self, Compact, Widget};

const TIMER_FRAME: usize = 1;
/// How often to check whether a fullscreen app has the foreground.
const FULLSCREEN_POLL: Duration = Duration::from_millis(500);
/// How long the notch pops open when a widget asks for attention.
const PEEK: Duration = Duration::from_secs(6);

const WM_TRAY: u32 = WM_APP + 1;
const TRAY_ID: u32 = 1;
const CMD_PIN: usize = 1;
const CMD_FULLSCREEN: usize = 2;
const CMD_AUTOSTART: usize = 3;
const CMD_QUIT: usize = 4;

// Logical (96-DPI) sizes.
const COLLAPSED: (f32, f32) = (190.0, 32.0);
const EXPANDED: (f32, f32) = (720.0, 240.0);
const RADIUS: f32 = 24.0;
/// Slack around the notch so the cursor grazing its edge doesn't flicker it.
const HOVER_MARGIN: f32 = 6.0;
/// Room around the expanded notch for its drop shadow.
const SHADOW: f32 = 18.0;

// Expanded layout, in logical units relative to the expanded notch's top-left.
const PAD: f32 = 24.0;
const TAB_Y: f32 = 8.0;
const TAB_W: f32 = 44.0;
const TAB_H: f32 = 32.0;
const TAB_GAP: f32 = 4.0;
const TAB_RADIUS: f32 = 8.0;
/// Where the line the tabs stand on is.
const TAB_BASE: f32 = TAB_Y + TAB_H;
const PIN_SIZE: f32 = 24.0;
const PIN: Rect = Rect::new(EXPANDED.0 - PAD - PIN_SIZE, TAB_Y + (TAB_H - PIN_SIZE) / 2.0, PIN_SIZE, PIN_SIZE);
const CONTENT: Rect = Rect::new(PAD, 48.0, EXPANDED.0 - PAD * 2.0, EXPANDED.1 - 48.0 - 16.0);
// Home dashboard cards.
const CARD_GAP: f32 = 12.0;
const CARD_PAD: f32 = 14.0;
const CARD_RADIUS: f32 = 16.0;
/// A slide that ends this close to the middle of the screen snaps to it.
const SNAP: f32 = 12.0;

/// Tab 0 is the Home dashboard; widget `i` is tab `i + 1`; the Settings tab is last, on the
/// right (see `settings_tab`).
const HOME: usize = 0;

fn settings_tab(widgets: &[Box<dyn Widget>]) -> usize {
    widgets.len() + 1
}

fn tab_count(widgets: &[Box<dyn Widget>]) -> usize {
    widgets.len() + 2
}

fn tab_rect(widgets: &[Box<dyn Widget>], tab: usize) -> Rect {
    if tab == settings_tab(widgets) {
        Rect::new(PIN.x - 8.0 - TAB_W, TAB_Y, TAB_W, TAB_H)
    } else {
        Rect::new(PAD + tab as f32 * (TAB_W + TAB_GAP), TAB_Y, TAB_W, TAB_H)
    }
}

fn tab_title(widgets: &[Box<dyn Widget>], tab: usize) -> &'static str {
    match tab {
        HOME => "Home",
        t if t == settings_tab(widgets) => "Settings",
        t => widgets[t - 1].title(),
    }
}

fn tab_icon(widgets: &[Box<dyn Widget>], tab: usize) -> &'static str {
    match tab {
        HOME => glyph::HOME,
        t if t == settings_tab(widgets) => glyph::GEAR,
        t => widgets[t - 1].icon(),
    }
}

/// The tab to restore by title. Never the Settings tab, which only opens when clicked.
fn tab_named(widgets: &[Box<dyn Widget>], title: &str) -> Option<usize> {
    (0..=widgets.len()).find(|&t| tab_title(widgets, t) == title)
}

/// The empty stretch of the tab row between the widget tabs and the Settings tab. Dragging
/// it slides the notch along the top of the screen.
fn handle_rect(widgets: &[Box<dyn Widget>]) -> Rect {
    let x = tab_rect(widgets, widgets.len()).right() + TAB_GAP;
    Rect::new(x, 0.0, tab_rect(widgets, settings_tab(widgets)).x - TAB_GAP - x, TAB_BASE)
}

/// Home dashboard cards as (widget index, rect in content coordinates), sized by weight.
fn home_cards(widgets: &[Box<dyn Widget>]) -> Vec<(usize, Rect)> {
    let weighted: Vec<(usize, f32)> =
        widgets.iter().enumerate().filter_map(|(i, w)| w.card_weight().map(|k| (i, k))).collect();
    let total: f32 = weighted.iter().map(|(_, k)| k).sum();
    let free = CONTENT.w - CARD_GAP * weighted.len().saturating_sub(1) as f32;
    let mut x = 0.0;
    weighted
        .into_iter()
        .map(|(i, k)| {
            let w = free * k / total;
            let card = (i, Rect::new(x, 0.0, w, CONTENT.h));
            x += w + CARD_GAP;
            card
        })
        .collect()
}

/// Broadcast by Explorer when the taskbar is (re)created; the tray icon must be re-added.
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);

/// 32bpp top-down DIB selected into a memory DC; these are the pixels pushed by
/// `UpdateLayeredWindow`.
struct Surface {
    dc: HDC,
    bitmap: HBITMAP,
    old: HGDIOBJ,
}

impl Surface {
    unsafe fn new(w: i32, h: i32) -> Result<Self> {
        unsafe {
            let screen_dc = GetDC(None);
            let dc = CreateCompatibleDC(Some(screen_dc));
            let bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w,
                    biHeight: -h, // top-down
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits = std::ptr::null_mut();
            let bitmap = CreateDIBSection(Some(screen_dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0);
            ReleaseDC(None, screen_dc);
            let bitmap = match bitmap {
                Ok(b) => b,
                Err(e) => {
                    let _ = DeleteDC(dc);
                    return Err(e);
                }
            };
            let old = SelectObject(dc, bitmap.into());
            Ok(Self { dc, bitmap, old })
        }
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old);
            let _ = DeleteObject(self.bitmap.into());
            let _ = DeleteDC(self.dc);
        }
    }
}

/// Geometry and resources that depend on the primary monitor's size and DPI.
/// Rebuilt on `WM_DISPLAYCHANGE` / `WM_DPICHANGED`.
struct Layout {
    scale: f32,
    /// Window size (the expanded footprint plus shadow room), in physical px.
    w: i32,
    h: i32,
    /// The primary monitor's bounds: where the notch slides, and for fullscreen detection.
    monitor: RECT,
    surface: Surface,
    fonts: Fonts,
}

/// Where the notch is this frame, in physical px.
struct Placement {
    /// The window's screen position.
    x: i32,
    y: i32,
    /// Window-relative left edge of the body, which slides as the notch animates.
    body: f32,
    /// Window-relative x of the expanded layout's origin.
    origin: f32,
}

/// Screen x of the body's left edge when collapsed and when expanded, at `position` along a
/// monitor spanning `left..left + width` (0 = collapsed notch flush left, 1 = flush right).
/// Both centre on the same point, except that the expanded notch keeps its shadow on-screen.
fn edges(left: f32, width: f32, scale: f32, position: f32) -> (f32, f32) {
    let (cw, ew, shadow) = (COLLAPSED.0 * scale, EXPANDED.0 * scale, SHADOW * scale);
    let centre = left + cw / 2.0 + position.clamp(0.0, 1.0) * (width - cw);
    let collapsed = (centre - cw / 2.0).clamp(left, (left + width - cw).max(left));
    let expanded = (centre - ew / 2.0).clamp(left + shadow, (left + width - ew - shadow).max(left + shadow));
    (collapsed, expanded)
}

impl Layout {
    /// At the top of the primary monitor.
    unsafe fn primary(dwrite: &IDWriteFactory) -> Result<Self> {
        unsafe {
            let mon = MonitorFromPoint(POINT::default(), MONITOR_DEFAULTTOPRIMARY);
            let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
            let _ = GetMonitorInfoW(mon, &mut info);
            let (mut dpi_x, mut dpi_y) = (96, 96);
            let _ = GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);
            let scale = dpi_x as f32 / 96.0;

            let w = ((EXPANDED.0 + SHADOW * 2.0) * scale).ceil() as i32;
            let h = ((EXPANDED.1 + SHADOW) * scale).ceil() as i32;

            Ok(Self {
                scale,
                w,
                h,
                monitor: info.rcMonitor,
                surface: Surface::new(w, h)?,
                fonts: Fonts::new(dwrite, scale)?,
            })
        }
    }

    /// How far the notch's centre can travel, in physical px.
    fn travel(&self) -> f32 {
        ((self.monitor.right - self.monitor.left) as f32 - COLLAPSED.0 * self.scale).max(1.0)
    }

    /// The window sits around the expanded notch; the fixed-size expanded layout stays put
    /// there (clipped to the body) while the body animates between its two positions.
    fn place(&self, position: f32, t: f32) -> Placement {
        let m = &self.monitor;
        let (collapsed, expanded) = edges(m.left as f32, (m.right - m.left) as f32, self.scale, position);
        let x = (expanded - SHADOW * self.scale).round() as i32;
        let body = lerp(collapsed, expanded, ease(t));
        Placement { x, y: m.top, body: body - x as f32, origin: expanded - x as f32 }
    }
}

unsafe fn create_target(d2d: &ID2D1Factory, look: Look) -> Result<(ID2D1DCRenderTarget, Brushes)> {
    unsafe {
        let props = D2D1_RENDER_TARGET_PROPERTIES {
            r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            usage: D2D1_RENDER_TARGET_USAGE_NONE,
            minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
        };
        let rt = d2d.CreateDCRenderTarget(&props)?;
        rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
        let brushes = Brushes::new(&rt, look)?;
        Ok((rt, brushes))
    }
}

/// A mouse drag in progress; the window has captured the mouse.
#[derive(Clone, Copy)]
enum Drag {
    /// Sliding the notch along the top of the screen, from the cursor's screen x and the
    /// notch's position when the drag began.
    Slide { from_x: i32, from: f32 },
    /// Setting opacity with the Settings tab's slider.
    Opacity,
    /// A drag a widget started from its `press` (by index into `widgets`).
    Widget(usize),
}

/// Something a click asks for that runs a modal loop, so must happen outside the `APP` borrow.
enum Modal {
    PickColor,
}

struct App {
    hwnd: HWND,
    d2d: ID2D1Factory,
    dwrite: IDWriteFactory,
    rt: ID2D1DCRenderTarget,
    brushes: Brushes,
    images: ImageCache,
    layout: Layout,
    widgets: Vec<Box<dyn Widget>>,
    /// The tab shown when expanded: `HOME`, or widget `i` as `i + 1`.
    active: usize,
    /// Cursor in expanded-layout logical coordinates while it is over the notch.
    cursor: Option<(f32, f32)>,
    /// 0.0 = collapsed, 1.0 = expanded.
    t: f32,
    target: f32,
    settings: Settings,
    /// A fullscreen app has the foreground, so the notch is faded out and hidden.
    hidden: bool,
    /// Window opacity: 1.0 = shown, 0.0 = faded out.
    fade: f32,
    next_fullscreen_check: Instant,
    /// Keeps the notch open while its own context menu or colour picker is showing.
    modal_open: bool,
    drag: Option<Drag>,
    /// Keeps the notch open until then, after a widget asked for attention.
    peek_until: Option<Instant>,
    /// The tray (and window) icon we drew; `None` falls back to the stock application icon.
    icon: Option<HICON>,
    /// False while the window is click-through (`WS_EX_TRANSPARENT`), i.e. the cursor is
    /// not over the notch body.
    interactive: bool,
    dirty: bool,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

pub fn run() -> Result<()> {
    if !desktop::claim_single_instance() {
        return Ok(());
    }
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)?;
        TASKBAR_CREATED.store(RegisterWindowMessageW(w!("TaskbarCreated")), Ordering::Relaxed);

        let hinst = GetModuleHandleW(None)?;
        let class = w!("MyNotchWindow");
        let wc = WNDCLASSW {
            hInstance: hinst.into(),
            lpszClassName: class,
            lpfnWndProc: Some(wndproc),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            ..Default::default()
        };
        RegisterClassW(&wc);

        // Position and size come from `Layout` and are applied by `UpdateLayeredWindow`.
        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            class,
            w!("MyNotch"),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(hinst.into()),
            None,
        )?;

        let mut app = App::new(hwnd)?;
        app.render();
        app.refresh_icon(NIM_ADD);
        APP.with(|a| *a.borrow_mut() = Some(app));

        SetTimer(Some(hwnd), TIMER_FRAME, 16, None);
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        // Release D2D/GDI resources now rather than during thread-local teardown at exit.
        APP.with(|a| a.borrow_mut().take());
    }
    Ok(())
}

impl App {
    unsafe fn new(hwnd: HWND) -> Result<Self> {
        unsafe {
            let d2d: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let settings = Settings::load();
            let (rt, brushes) = create_target(&d2d, settings_panel::look(&settings))?;
            let layout = Layout::primary(&dwrite)?;
            let mut widgets = widgets::all();
            for w in &mut widgets {
                w.load(&settings);
            }
            let active = tab_named(&widgets, &settings.tab).unwrap_or(HOME);

            Ok(Self {
                hwnd,
                d2d,
                dwrite,
                rt,
                brushes,
                images: ImageCache::default(),
                layout,
                widgets,
                active,
                cursor: None,
                t: 0.0,
                target: 0.0,
                settings,
                hidden: false,
                fade: 1.0,
                next_fullscreen_check: Instant::now(),
                peek_until: None,
                icon: None,
                modal_open: false,
                drag: None,
                interactive: true,
                dirty: true,
            })
        }
    }

    /// Re-fit to the primary monitor after a resolution, DPI or arrangement change.
    fn relayout(&mut self) {
        if let Ok(layout) = unsafe { Layout::primary(&self.dwrite) } {
            self.layout = layout;
            self.render();
            self.refresh_icon(NIM_MODIFY);
        }
    }

    /// Redraws the icon for the current DPI and taskbar theme, then adds (`NIM_ADD`) or
    /// updates (`NIM_MODIFY`) the tray icon and sets it as the window icon too.
    fn refresh_icon(&mut self, op: NOTIFY_ICON_MESSAGE) {
        let dpi = (self.layout.scale * 96.0).round() as u32;
        let size = unsafe { GetSystemMetricsForDpi(SM_CXSMICON, dpi) };
        if let Ok(icon) = icon::render(&self.d2d, size, desktop::taskbar_is_light()) {
            // Hand out the new icon before destroying the old one, so nothing shows a dead handle.
            tray_notify(self.hwnd, op, icon);
            unsafe {
                for kind in [ICON_SMALL, ICON_BIG] {
                    SendMessageW(self.hwnd, WM_SETICON, Some(WPARAM(kind as usize)), Some(LPARAM(icon.0 as isize)));
                }
                if let Some(old) = self.icon.replace(icon) {
                    let _ = DestroyIcon(old);
                }
            }
        } else {
            let fallback = self.icon.or_else(|| unsafe { LoadIconW(None, IDI_APPLICATION) }.ok());
            tray_notify(self.hwnd, op, fallback.unwrap_or_default());
        }
    }

    fn toggle_pin(&mut self) {
        self.settings.pinned = !self.settings.pinned;
        self.settings.save();
        self.dirty = true;
    }

    fn toggle_hide_in_fullscreen(&mut self) {
        self.settings.hide_in_fullscreen = !self.settings.hide_in_fullscreen;
        self.settings.save();
        // Re-evaluate on the next frame rather than waiting for the next poll.
        self.next_fullscreen_check = Instant::now();
    }

    fn select_tab(&mut self, i: usize) {
        if i != self.active {
            self.active = i;
            // Settings is never reopened by itself, so isn't remembered.
            if i != settings_tab(&self.widgets) {
                self.settings.tab = tab_title(&self.widgets, i).to_string();
                self.settings.save();
            }
            self.dirty = true;
        }
    }

    /// Repaints the body with a new colour and opacity. The caller saves the settings.
    fn set_look(&mut self, color: u32, opacity: u32) {
        if (color, opacity) != (self.settings.color, self.settings.opacity) {
            self.settings.color = color;
            self.settings.opacity = opacity;
            let _ = unsafe { self.brushes.restyle(&self.rt, settings_panel::look(&self.settings)) };
            self.dirty = true;
        }
    }

    fn placement(&self) -> Placement {
        self.layout.place(self.settings.position, self.t)
    }

    /// The cursor in expanded-layout logical coordinates.
    fn cursor_logical(&self) -> (f32, f32) {
        let mut pt = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut pt);
        }
        let (p, s) = (self.placement(), self.layout.scale);
        (((pt.x - p.x) as f32 - p.origin) / s, (pt.y - p.y) as f32 / s)
    }

    /// Whether the cursor is over the open notch's slide handle.
    fn over_handle(&self) -> bool {
        let (x, y) = self.cursor_logical();
        self.t >= 0.9 && handle_rect(&self.widgets).contains(x, y)
    }

    /// The collapsed readout, and the index of the widget providing it.
    fn compact(&self) -> Option<(usize, Compact)> {
        self.widgets
            .iter()
            .enumerate()
            .filter_map(|(i, w)| w.compact().map(|c| (i, c)))
            .max_by_key(|(_, c)| c.priority)
    }

    /// Current notch size in physical pixels.
    fn notch_size(&self) -> (f32, f32) {
        let e = ease(self.t);
        let w = lerp(COLLAPSED.0, EXPANDED.0, e) * self.layout.scale;
        let h = lerp(COLLAPSED.1, EXPANDED.1, e) * self.layout.scale;
        (w, h)
    }

    /// Per-frame: widget updates, hover detection and animation.
    fn tick(&mut self) {
        let now = Instant::now();
        let mut updated = false;
        for w in &mut self.widgets {
            updated |= w.update(now);
        }
        if updated {
            self.dirty = true;
            self.persist_widgets();
        }
        if let Some(i) = self.widgets.iter_mut().position(|w| w.take_attention()) {
            // E.g. a focus session ended: pop open on that widget, with a chime.
            self.active = i + 1;
            self.peek_until = Some(now + PEEK);
            unsafe {
                let _ = PlaySoundW(w!("Notification.Reminder"), None, SND_ALIAS | SND_ASYNC);
            }
        }
        if self.peek_until.is_some_and(|until| now >= until) {
            self.peek_until = None;
        }

        // The notch is never the foreground window, so its mouse capture only sees the
        // cursor while it's over the window: follow the drag here too, and end it once the
        // button is up even if the release happened elsewhere.
        if self.drag.is_some() {
            if primary_button_down() {
                self.on_drag();
            } else {
                self.end_drag();
            }
        }

        // Fade out and hide while a fullscreen app has the foreground; come back after.
        if now >= self.next_fullscreen_check {
            self.next_fullscreen_check = now + FULLSCREEN_POLL;
            let hide = self.settings.hide_in_fullscreen && desktop::foreground_is_fullscreen(self.layout.monitor);
            if hide != self.hidden {
                self.hidden = hide;
                if !hide {
                    unsafe {
                        let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
                    }
                }
            }
        }
        let fade_target = if self.hidden { 0.0 } else { 1.0 };
        if self.fade != fade_target {
            self.fade = approach(self.fade, fade_target, 0.3);
            self.dirty = true;
            if self.fade == 0.0 {
                // Fully faded: hide the window itself, then stop drawing until it returns.
                unsafe {
                    let _ = ShowWindow(self.hwnd, SW_HIDE);
                }
                self.dirty = false;
            }
        }
        if self.hidden && self.fade == 0.0 {
            return;
        }

        let mut pt = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut pt);
        }
        let s = self.layout.scale;
        let p = self.placement();
        let (nw, nh) = self.notch_size();
        let m = HOVER_MARGIN * s;
        let (body_left, top) = (p.x as f32 + p.body, p.y as f32);
        let (px, py) = (pt.x as f32, pt.y as f32);
        let dragging = self.drag.is_some();
        let hovered = dragging
            || !self.hidden && px >= body_left - m && px < body_left + nw + m && py >= top && py < top + nh + m;

        // Only the body takes clicks; the rest of the window, shadow included, lets them
        // through to whatever is underneath. A drag keeps the mouse wherever it wanders.
        let over_body = dragging || in_notch(px - body_left, py - top, nw, nh, RADIUS * s);
        if over_body != self.interactive {
            self.interactive = over_body;
            set_click_through(self.hwnd, !over_body);
        }

        let cursor = hovered.then(|| ((px - p.x as f32 - p.origin) / s, (py - top) / s));
        if cursor != self.cursor {
            self.cursor = cursor;
            if self.t > 0.5 {
                self.dirty = true;
            }
        }

        let peeking = self.peek_until.is_some();
        let target = if hovered || self.settings.pinned || self.modal_open || peeking { 1.0 } else { 0.0 };
        if target > self.target && self.t < 0.5 {
            // Opening from collapsed: Home if that's where the user left it (it shows the live
            // activity anyway); else live activity (music, a running timer) takes the stage;
            // else the tab the user last picked; else Home.
            self.active = match (tab_named(&self.widgets, &self.settings.tab), self.compact()) {
                (Some(HOME), _) => HOME,
                (_, Some((i, c))) if c.live => i + 1,
                (saved, _) => saved.unwrap_or(HOME),
            };
        }
        self.target = target;

        if self.t != target {
            self.t = approach(self.t, target, 0.22);
            self.dirty = true;
        }
        if self.dirty {
            self.render();
        }
    }

    /// Left button down at window-relative physical coordinates.
    fn on_press(&mut self, x: i32, y: i32) -> Option<Modal> {
        if self.t < 0.9 {
            return None;
        }
        let s = self.layout.scale;
        let (lx, ly) = ((x as f32 - self.placement().origin) / s, y as f32 / s);

        if PIN.contains(lx, ly) {
            self.toggle_pin();
        } else if let Some(t) = (0..tab_count(&self.widgets)).find(|&t| tab_rect(&self.widgets, t).contains(lx, ly)) {
            self.select_tab(t);
        } else if handle_rect(&self.widgets).contains(lx, ly) {
            // Where the press happened, on screen (the cursor may have moved on since).
            let from_x = self.placement().x + x;
            self.begin_drag(Drag::Slide { from_x, from: self.settings.position });
        } else if CONTENT.contains(lx, ly) && self.active == settings_tab(&self.widgets) {
            let (cx, cy) = (lx - CONTENT.x, ly - CONTENT.y);
            match settings_panel::hit(cx, cy)? {
                Hit::Preset(color) => {
                    self.set_look(color, self.settings.opacity);
                    self.settings.save();
                }
                Hit::Custom => return Some(Modal::PickColor),
                Hit::Opacity => {
                    self.set_look(self.settings.color, settings_panel::opacity_at(cx));
                    self.begin_drag(Drag::Opacity);
                }
                Hit::Center => {
                    self.settings.position = 0.5;
                    self.settings.save();
                    self.dirty = true;
                }
            }
        } else if CONTENT.contains(lx, ly) {
            let (cx, cy) = (lx - CONTENT.x, ly - CONTENT.y);
            let handled = if self.active == HOME {
                home_cards(&self.widgets).into_iter().find(|(_, r)| r.contains(cx, cy)).map(|(i, r)| {
                    let (x, y) = (cx - r.x - CARD_PAD, cy - r.y - CARD_PAD);
                    (i, self.widgets[i].click_card(x, y, r.w - CARD_PAD * 2.0, r.h - CARD_PAD * 2.0))
                })
            } else {
                let i = self.active - 1;
                if self.widgets[i].press(cx, cy, CONTENT.w, CONTENT.h) {
                    self.begin_drag(Drag::Widget(i));
                    return None;
                }
                Some((i, self.widgets[i].click(cx, cy, CONTENT.w, CONTENT.h)))
            };
            if let Some((_, true)) = handled {
                self.dirty = true;
                self.persist_widgets();
            }
        }
        None
    }

    fn begin_drag(&mut self, drag: Drag) {
        unsafe { SetCapture(self.hwnd) };
        self.drag = Some(drag);
        self.dirty = true;
    }

    /// The mouse moved while the window has it captured.
    fn on_drag(&mut self) {
        let Some(drag) = self.drag else { return };
        let mut pt = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut pt);
        }
        match drag {
            Drag::Slide { from_x, from } => {
                let travel = self.layout.travel();
                let mut position = (from + (pt.x - from_x) as f32 / travel).clamp(0.0, 1.0);
                if ((position - 0.5) * travel).abs() < SNAP * self.layout.scale {
                    position = 0.5;
                }
                if position != self.settings.position {
                    self.settings.position = position;
                    self.dirty = true;
                }
            }
            Drag::Opacity => {
                let (x, _) = self.cursor_logical();
                self.set_look(self.settings.color, settings_panel::opacity_at(x - CONTENT.x));
            }
            Drag::Widget(i) => {
                let (x, y) = self.cursor_logical();
                if self.widgets[i].drag(x - CONTENT.x, y - CONTENT.y) {
                    self.dirty = true;
                }
            }
        }
    }

    /// The button came up: keep what the drag set.
    fn end_drag(&mut self) {
        let Some(drag) = self.drag.take() else { return };
        unsafe {
            let _ = ReleaseCapture();
        }
        if let Drag::Widget(i) = drag {
            let (x, y) = self.cursor_logical();
            self.widgets[i].release(x - CONTENT.x, y - CONTENT.y);
            self.persist_widgets();
        } else {
            self.settings.save();
        }
        self.dirty = true;
    }

    /// Saves the settings file if any widget's preferences changed.
    fn persist_widgets(&mut self) {
        let before = self.settings.clone();
        for w in &self.widgets {
            w.store(&mut self.settings);
        }
        if self.settings != before {
            self.settings.save();
        }
    }

    /// Mouse wheel, in notches: goes to the open panel. (Windows only sends it while the
    /// cursor is over the notch.)
    fn on_wheel(&mut self, notches: f32) {
        if self.t < 0.9 || self.active == HOME {
            return;
        }
        if self.active == settings_tab(&self.widgets) {
            let (x, y) = self.cursor_logical();
            let (opacity, color) = (self.settings.opacity, self.settings.color);
            if let Some(o) = settings_panel::wheel_opacity(x - CONTENT.x, y - CONTENT.y, opacity, notches) {
                self.set_look(color, o);
                self.settings.save();
            }
        } else if self.widgets[self.active - 1].wheel(notches) {
            self.dirty = true;
        }
    }

    fn render(&mut self) {
        self.dirty = false;
        let l = &self.layout;
        let s = l.scale;
        let (nw, nh) = self.notch_size();
        let p = self.placement();
        let left = p.body;
        let e = ease(self.t);

        unsafe {
            let bind = RECT { left: 0, top: 0, right: l.w, bottom: l.h };
            if self.rt.BindDC(l.surface.dc, &bind).is_err() {
                return;
            }
            self.rt.BeginDraw();
            self.rt.Clear(Some(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }));

            // Body: extend above the window so the top corners stay flat. The shadow fades in
            // with expansion and is absent when collapsed, keeping click-through exact.
            let canvas =
                |origin: f32, cursor| Canvas::new(&self.rt, &self.brushes, &l.fonts, &self.images, s, (origin, 0.0), cursor);
            let body = canvas(left, None);
            body.glass(Rect::new(0.0, -RADIUS, nw / s, nh / s + RADIUS), RADIUS, e);

            // Collapsed content: the top-priority widget's one-line readout.
            if e < 0.5
                && let Some((_, compact)) = self.compact()
            {
                self.brushes.set_content_opacity(1.0 - e * 2.0);
                let r = Rect::new(14.0, 0.0, nw / s - 28.0, nh / s);
                body.text(&compact.text, Font::Small, Ink::Dim, r, Align::Center);
            }

            // Expanded content: fixed layout, clipped to the animating body.
            if e > 0.5 {
                self.brushes.set_content_opacity((e - 0.5) * 2.0);
                let ox = p.origin;
                let cv = canvas(ox, self.cursor);
                cv.push_clip(Rect::new((left - ox) / s, 0.0, nw / s, nh / s));
                self.draw_tabs(&cv);

                let content = cv.offset(CONTENT.x, CONTENT.y);
                if self.active == HOME {
                    for (i, r) in home_cards(&self.widgets) {
                        content.fill(r, CARD_RADIUS, Ink::Track);
                        let inner = content.offset(r.x + CARD_PAD, r.y + CARD_PAD);
                        self.widgets[i].draw_card(&inner, r.w - CARD_PAD * 2.0, r.h - CARD_PAD * 2.0);
                    }
                } else if self.active == settings_tab(&self.widgets) {
                    settings_panel::draw(&content, CONTENT.w, &self.settings);
                } else {
                    self.widgets[self.active - 1].draw(&content, CONTENT.w, CONTENT.h);
                }
                cv.pop_clip();
            }

            match self.rt.EndDraw(None, None) {
                Ok(()) => {}
                Err(err) if err.code() == D2DERR_RECREATE_TARGET => {
                    // Device lost: rebuild the target and brushes, redraw next tick.
                    if let Ok((rt, brushes)) = create_target(&self.d2d, settings_panel::look(&self.settings)) {
                        self.rt = rt;
                        self.brushes = brushes;
                        self.images.clear();
                    }
                    self.dirty = true;
                    return;
                }
                Err(_) => return,
            }

            let dst = POINT { x: p.x, y: p.y };
            let size = SIZE { cx: l.w, cy: l.h };
            let src = POINT::default();
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: (self.fade * 255.0).round() as u8,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            let _ = UpdateLayeredWindow(
                self.hwnd,
                None,
                Some(&dst),
                Some(&size),
                Some(l.surface.dc),
                Some(&src),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            );
        }
    }

    /// The tab row: outlined tabs standing on a line that opens under the open tab, so the
    /// tab joins the panel below it; the name of the tab under the cursor (else the open one);
    /// the slide handle's grip; and the pin.
    fn draw_tabs(&self, cv: &Canvas) {
        let widgets = &self.widgets;
        let active = tab_rect(widgets, self.active);
        let y = TAB_BASE - 0.5;
        cv.line((PAD / 2.0, y), (active.x + 0.5, y), Ink::Edge);
        cv.line((active.right() - 0.5, y), (EXPANDED.0 - PAD / 2.0, y), Ink::Edge);

        let mut named = self.active;
        for t in 0..tab_count(widgets) {
            let r = tab_rect(widgets, t);
            let hot = cv.hovered(r);
            if hot {
                named = t;
            }
            let (fill, edge, ink) = match (t == self.active, hot) {
                (true, _) => (Some(Ink::Track), Ink::Edge, Ink::Text),
                (false, true) => (Some(Ink::Track), Ink::Line, Ink::Text),
                (false, false) => (None, Ink::Line, Ink::Dim),
            };
            cv.tab(r, TAB_RADIUS, fill, Some(edge));
            cv.text(tab_icon(widgets, t), Font::Icon, ink, r, Align::Center);
        }

        let handle = handle_rect(widgets);
        // The name stops short of the grip in the middle.
        let label = Rect::new(handle.x + 8.0, TAB_Y, handle.w / 2.0 - 20.0, TAB_H);
        cv.text(tab_title(widgets, named), Font::Label, Ink::Dim, label, Align::Left);
        // Grip: two rows of three dots, centred in the handle.
        let hot = matches!(self.drag, Some(Drag::Slide { .. })) || cv.hovered(handle);
        let (cx, cy) = (handle.x + handle.w / 2.0, TAB_Y + TAB_H / 2.0);
        for (dx, dy) in [-5.0, 0.0, 5.0].into_iter().flat_map(|dx| [(dx, -2.5), (dx, 2.5)]) {
            cv.circle(cx + dx, cy + dy, 1.25, if hot { Ink::Text } else { Ink::Dim }, None);
        }

        let (pin, ink) = match (self.settings.pinned, cv.hovered(PIN)) {
            (true, _) => (glyph::PINNED, Ink::Accent),
            (false, true) => (glyph::PIN, Ink::Text),
            (false, false) => (glyph::PIN, Ink::Dim),
        };
        cv.text(pin, Font::Icon, ink, PIN, Align::Center);
    }
}

/// Whether (`x`, `y`), relative to the body's top-left, is inside a `w` x `h` notch whose
/// bottom corners are rounded by `r` (the top corners are off-screen).
fn in_notch(x: f32, y: f32, w: f32, h: f32, r: f32) -> bool {
    if x < 0.0 || x >= w || y < 0.0 || y >= h {
        return false;
    }
    let cy = h - r;
    let cx = if x < r { r } else if x > w - r { w - r } else { return true };
    if y <= cy {
        return true;
    }
    let (dx, dy) = (x - cx, y - cy);
    dx * dx + dy * dy <= r * r
}

/// Whether the primary mouse button is physically down now, wherever the cursor is.
fn primary_button_down() -> bool {
    // GetAsyncKeyState reads the physical buttons, so honour "switch primary and secondary".
    let swapped = unsafe { GetSystemMetrics(SM_SWAPBUTTON) } != 0;
    let button = if swapped { VK_RBUTTON } else { VK_LBUTTON };
    let state = unsafe { GetAsyncKeyState(i32::from(button.0)) };
    state < 0
}

fn set_click_through(hwnd: HWND, on: bool) {
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let flag = WS_EX_TRANSPARENT.0 as isize;
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, if on { style | flag } else { style & !flag });
    }
}

/// Eases `v` a fraction `k` of the way to `target`, snapping once close.
fn approach(v: f32, target: f32, k: f32) -> f32 {
    let next = v + (target - v) * k;
    if (next - target).abs() < 0.005 { target } else { next }
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Smoothstep.
fn ease(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

fn with_app(f: impl FnOnce(&mut App)) {
    APP.with(|a| {
        if let Ok(mut guard) = a.try_borrow_mut()
            && let Some(app) = guard.as_mut()
        {
            f(app);
        }
    });
}

/// Adds, updates or removes (`NIM_DELETE`, where `icon` is ignored) the tray icon.
fn tray_notify(hwnd: HWND, op: NOTIFY_ICON_MESSAGE, icon: HICON) {
    let mut nid = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: TRAY_ID,
        uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
        uCallbackMessage: WM_TRAY,
        hIcon: icon,
        ..Default::default()
    };
    for (dst, src) in nid.szTip.iter_mut().zip("MyNotch".encode_utf16()) {
        *dst = src;
    }
    unsafe {
        let _ = Shell_NotifyIconW(op, &nid);
    }
}

/// Shows the settings/quit menu at the cursor. `TrackPopupMenu` runs a modal loop, so this
/// must be called without the `APP` borrow held, letting the frame timer keep animating.
fn show_menu(hwnd: HWND, from_notch: bool) {
    let mut settings = Settings::default();
    with_app(|a| {
        settings = a.settings.clone();
        a.modal_open = from_notch;
    });

    let cmd = unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        let check = |on: bool| if on { MF_STRING | MF_CHECKED } else { MF_STRING };
        let _ = AppendMenuW(menu, check(settings.pinned), CMD_PIN, w!("Pin open"));
        let _ = AppendMenuW(menu, check(settings.hide_in_fullscreen), CMD_FULLSCREEN, w!("Hide over fullscreen apps"));
        let _ = AppendMenuW(menu, check(autostart::enabled()), CMD_AUTOSTART, w!("Start with Windows"));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(menu, MF_STRING, CMD_QUIT, w!("Quit MyNotch"));

        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        // Without this the menu won't dismiss when the user clicks elsewhere.
        let _ = SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
            pt.x,
            pt.y,
            None,
            hwnd,
            None,
        );
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        cmd.0 as usize
    };

    with_app(|a| a.modal_open = false);
    match cmd {
        CMD_PIN => with_app(|a| a.toggle_pin()),
        CMD_FULLSCREEN => with_app(|a| a.toggle_hide_in_fullscreen()),
        CMD_AUTOSTART => {
            autostart::set(!autostart::enabled());
        }
        CMD_QUIT => unsafe {
            let _ = DestroyWindow(hwnd);
        },
        _ => {}
    }
}

/// Opens Windows' colour picker under the notch, which previews each colour tried and keeps
/// the chosen one (or goes back on Cancel). Like `show_menu`, it runs a modal loop, so must be
/// called without the `APP` borrow held.
fn pick_color(hwnd: HWND) {
    let mut start = None;
    with_app(|a| {
        a.modal_open = true;
        let (p, s, m) = (a.placement(), a.layout.scale, a.layout.monitor);
        let picker = settings_panel::Picker {
            centre: p.x + (p.origin + EXPANDED.0 * s / 2.0) as i32,
            top: p.y + ((EXPANDED.1 + 8.0) * s) as i32,
            left: m.left,
            right: m.right,
            preview: |color| with_app(|a| a.set_look(color, a.settings.opacity)),
        };
        start = Some((a.settings.color, picker));
    });
    let Some((initial, picker)) = start else { return };
    let picked = settings_panel::pick_color(hwnd, initial, picker);
    with_app(|a| {
        a.modal_open = false;
        a.set_look(picked.unwrap_or(initial), a.settings.opacity);
        a.settings.save();
    });
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_TIMER => {
            if wparam.0 == TIMER_FRAME {
                with_app(|a| a.tick());
            }
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let x = (lparam.0 & 0xFFFF) as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
            let mut modal = None;
            with_app(|a| modal = a.on_press(x, y));
            if let Some(Modal::PickColor) = modal {
                pick_color(hwnd);
            }
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            with_app(|a| a.on_drag());
            LRESULT(0)
        }
        // A lost capture doesn't end a drag: `tick` follows it until the button is up.
        WM_LBUTTONUP => {
            with_app(|a| a.end_drag());
            LRESULT(0)
        }
        // A sideways-arrows cursor over the slide handle.
        WM_SETCURSOR if (lparam.0 & 0xFFFF) as u32 == HTCLIENT => {
            let mut over = false;
            with_app(|a| over = a.over_handle());
            if over && let Ok(cursor) = unsafe { LoadCursorW(None, IDC_SIZEWE) } {
                unsafe { SetCursor(Some(cursor)) };
                return LRESULT(1);
            }
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        WM_RBUTTONUP => {
            show_menu(hwnd, true);
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            let delta = ((wparam.0 >> 16) & 0xFFFF) as u16 as i16;
            with_app(|a| a.on_wheel(f32::from(delta) / WHEEL_DELTA as f32));
            LRESULT(0)
        }
        WM_TRAY => {
            match (lparam.0 & 0xFFFF) as u32 {
                WM_LBUTTONUP => with_app(|a| a.toggle_pin()),
                WM_RBUTTONUP => show_menu(hwnd, false),
                _ => {}
            }
            LRESULT(0)
        }
        WM_DISPLAYCHANGE | WM_DPICHANGED => {
            with_app(|a| a.relayout());
            LRESULT(0)
        }
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        // Light/dark theme switched: redraw the tray icon to match the taskbar.
        WM_SETTINGCHANGE if lparam.0 != 0 && unsafe { PCWSTR(lparam.0 as *const u16).to_string() }
            .is_ok_and(|s| s == "ImmersiveColorSet") =>
        {
            with_app(|a| a.refresh_icon(NIM_MODIFY));
            LRESULT(0)
        }
        WM_DESTROY => {
            tray_notify(hwnd, NIM_DELETE, HICON::default());
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ if msg != 0 && msg == TASKBAR_CREATED.load(Ordering::Relaxed) => {
            with_app(|a| a.refresh_icon(NIM_ADD));
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

#[cfg(test)]
mod tests {
    use super::{COLLAPSED, EXPANDED, SHADOW, edges, in_notch};

    #[test]
    fn centred_notch_is_concentric_on_the_monitor() {
        let (collapsed, expanded) = edges(0.0, 1920.0, 1.0, 0.5);
        assert_eq!(collapsed + COLLAPSED.0 / 2.0, 960.0);
        assert_eq!(expanded + EXPANDED.0 / 2.0, 960.0);
        let (collapsed, expanded) = edges(-2560.0, 2560.0, 1.5, 0.5);
        assert_eq!(collapsed + COLLAPSED.0 * 1.5 / 2.0, -1280.0, "a monitor left of the primary, scaled");
        assert_eq!(expanded + EXPANDED.0 * 1.5 / 2.0, -1280.0);
    }

    #[test]
    fn notch_slides_flush_to_the_edges_but_the_expanded_one_keeps_its_shadow_on_screen() {
        let (collapsed, expanded) = edges(0.0, 1920.0, 1.0, 0.0);
        assert_eq!(collapsed, 0.0, "collapsed: flush left");
        assert_eq!(expanded, SHADOW, "expanded: shadow room to the left");
        let (collapsed, expanded) = edges(0.0, 1920.0, 1.0, 1.0);
        assert_eq!(collapsed + COLLAPSED.0, 1920.0, "collapsed: flush right");
        assert_eq!(expanded + EXPANDED.0 + SHADOW, 1920.0);
        // Partway: the collapsed notch moves on while the expanded one waits at the edge.
        let (collapsed, expanded) = edges(0.0, 1920.0, 1.0, 0.1);
        assert!(collapsed > 0.0 && expanded == SHADOW);
        assert!(collapsed >= expanded - SHADOW && collapsed + COLLAPSED.0 <= expanded + EXPANDED.0 + SHADOW, "inside the window");
    }

    #[test]
    fn notch_hit_area_follows_rounded_bottom_corners() {
        let (w, h, r) = (190.0, 32.0, 20.0);
        assert!(in_notch(95.0, 16.0, w, h, r), "centre");
        assert!(in_notch(1.0, 1.0, w, h, r), "top-left corner is square");
        assert!(in_notch(189.0, 1.0, w, h, r), "top-right corner is square");
        assert!(in_notch(95.0, 31.5, w, h, r), "bottom edge, middle");
        assert!(!in_notch(1.0, 31.0, w, h, r), "bottom-left corner is rounded off");
        assert!(!in_notch(189.0, 31.0, w, h, r), "bottom-right corner is rounded off");
        assert!(in_notch(6.0, 26.0, w, h, r), "just inside the bottom-left curve");
        assert!(!in_notch(-0.5, 10.0, w, h, r), "left of the body");
        assert!(!in_notch(95.0, 32.0, w, h, r), "below the body");
    }
}
