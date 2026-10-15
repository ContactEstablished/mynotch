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
use crate::gfx::{Align, Brushes, Canvas, Font, Fonts, Ink, Rect, glyph};
use crate::icon;
use crate::settings::Settings;
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
const EXPANDED: (f32, f32) = (520.0, 150.0);
const RADIUS: f32 = 20.0;
/// Slack around the notch so the cursor grazing its edge doesn't flicker it.
const HOVER_MARGIN: f32 = 6.0;
/// Room around the expanded notch for its drop shadow.
const SHADOW: f32 = 18.0;

// Expanded layout, in logical units relative to the expanded notch's top-left.
const PAD: f32 = 24.0;
const TAB_Y: f32 = 12.0;
const TAB_W: f32 = 72.0;
const TAB_H: f32 = 24.0;
const TAB_GAP: f32 = 4.0;
const PIN: Rect = Rect::new(EXPANDED.0 - PAD - TAB_H, TAB_Y, TAB_H, TAB_H);
const CONTENT: Rect = Rect::new(PAD, 44.0, EXPANDED.0 - PAD * 2.0, EXPANDED.1 - 44.0 - 12.0);

fn tab_rect(i: usize) -> Rect {
    Rect::new(PAD + i as f32 * (TAB_W + TAB_GAP), TAB_Y, TAB_W, TAB_H)
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
    /// Window origin and size (the expanded footprint plus shadow room), in physical px.
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    /// The primary monitor's bounds, for fullscreen detection.
    monitor: RECT,
    surface: Surface,
    fonts: Fonts,
}

impl Layout {
    /// Centred at the top of the primary monitor.
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
            let m = info.rcMonitor;

            Ok(Self {
                scale,
                x: m.left + (m.right - m.left - w) / 2,
                y: m.top,
                w,
                h,
                monitor: m,
                surface: Surface::new(w, h)?,
                fonts: Fonts::new(dwrite, scale)?,
            })
        }
    }

    /// Window-relative physical x of the expanded layout's origin. The fixed-size expanded
    /// layout stays centred (and clipped to the body) while the notch animates.
    fn origin_x(&self) -> f32 {
        (self.w as f32 - EXPANDED.0 * self.scale) / 2.0
    }
}

unsafe fn create_target(d2d: &ID2D1Factory) -> Result<(ID2D1DCRenderTarget, Brushes)> {
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
        let brushes = Brushes::new(&rt)?;
        Ok((rt, brushes))
    }
}

struct App {
    hwnd: HWND,
    d2d: ID2D1Factory,
    dwrite: IDWriteFactory,
    rt: ID2D1DCRenderTarget,
    brushes: Brushes,
    layout: Layout,
    widgets: Vec<Box<dyn Widget>>,
    /// Index of the widget shown when expanded.
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
    /// Keeps the notch open while its own context menu is showing.
    menu_open: bool,
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
            let (rt, brushes) = create_target(&d2d)?;
            let layout = Layout::primary(&dwrite)?;
            let settings = Settings::load();
            let mut widgets = widgets::all();
            for w in &mut widgets {
                w.load(&settings);
            }
            let active = widgets.iter().position(|w| w.title() == settings.tab).unwrap_or(0);

            Ok(Self {
                hwnd,
                d2d,
                dwrite,
                rt,
                brushes,
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
                menu_open: false,
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
            self.settings.tab = self.widgets[i].title().to_string();
            self.settings.save();
            self.dirty = true;
        }
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
        for w in &mut self.widgets {
            if w.update(now) {
                self.dirty = true;
            }
        }
        if let Some(i) = self.widgets.iter_mut().position(|w| w.take_attention()) {
            // E.g. a focus session ended: pop open on that widget, with a chime.
            self.active = i;
            self.peek_until = Some(now + PEEK);
            unsafe {
                let _ = PlaySoundW(w!("Notification.Reminder"), None, SND_ALIAS | SND_ASYNC);
            }
        }
        if self.peek_until.is_some_and(|until| now >= until) {
            self.peek_until = None;
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
        let l = &self.layout;
        let s = l.scale;
        let (nw, nh) = self.notch_size();
        let m = HOVER_MARGIN * s;
        let body_left = l.x as f32 + (l.w as f32 - nw) / 2.0;
        let left = body_left - m;
        let (px, py) = (pt.x as f32, pt.y as f32);
        let hovered = !self.hidden
            && px >= left
            && px < left + nw + m * 2.0
            && py >= l.y as f32
            && py < l.y as f32 + nh + m;

        // Only the body takes clicks; the rest of the window, shadow included, lets them
        // through to whatever is underneath.
        let over_body = in_notch(px - body_left, py - l.y as f32, nw, nh, RADIUS * s);
        if over_body != self.interactive {
            self.interactive = over_body;
            set_click_through(self.hwnd, !over_body);
        }

        let cursor = hovered.then(|| ((px - l.x as f32 - l.origin_x()) / s, (py - l.y as f32) / s));
        if cursor != self.cursor {
            self.cursor = cursor;
            if self.t > 0.5 {
                self.dirty = true;
            }
        }

        let peeking = self.peek_until.is_some();
        let target = if hovered || self.settings.pinned || self.menu_open || peeking { 1.0 } else { 0.0 };
        if target > self.target && self.t < 0.5 {
            // Opening from collapsed: live activity (e.g. music playing) takes the stage;
            // otherwise the tab the user last picked, else whatever the readout showed.
            let saved = self.widgets.iter().position(|w| w.title() == self.settings.tab);
            self.active = match self.compact() {
                Some((i, c)) if c.live => i,
                readout => saved.or(readout.map(|(i, _)| i)).unwrap_or(self.active),
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

    /// A left click at window-relative physical coordinates.
    fn on_click(&mut self, x: i32, y: i32) {
        if self.t < 0.9 {
            return;
        }
        let s = self.layout.scale;
        let (lx, ly) = ((x as f32 - self.layout.origin_x()) / s, y as f32 / s);

        if PIN.contains(lx, ly) {
            self.toggle_pin();
        } else if let Some(i) = (0..self.widgets.len()).find(|&i| tab_rect(i).contains(lx, ly)) {
            self.select_tab(i);
        } else if CONTENT.contains(lx, ly) {
            let (cx, cy) = (lx - CONTENT.x, ly - CONTENT.y);
            if self.widgets[self.active].click(cx, cy, CONTENT.w, CONTENT.h) {
                self.dirty = true;
                let before = self.settings.clone();
                self.widgets[self.active].store(&mut self.settings);
                if self.settings != before {
                    self.settings.save();
                }
            }
        }
    }

    fn render(&mut self) {
        self.dirty = false;
        let l = &self.layout;
        let s = l.scale;
        let (nw, nh) = self.notch_size();
        let left = (l.w as f32 - nw) / 2.0;
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
            let canvas = |origin: f32, cursor| Canvas::new(&self.rt, &self.brushes, &l.fonts, s, (origin, 0.0), cursor);
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

            // Expanded content: fixed layout, centred and clipped to the animating body.
            if e > 0.5 {
                self.brushes.set_content_opacity((e - 0.5) * 2.0);
                let ox = l.origin_x();
                let cv = canvas(ox, self.cursor);
                cv.push_clip(Rect::new((left - ox) / s, 0.0, nw / s, nh / s));

                for (i, w) in self.widgets.iter().enumerate() {
                    let r = tab_rect(i);
                    let ink = if i == self.active {
                        cv.fill(r, TAB_H / 2.0, Ink::Track);
                        Ink::Text
                    } else if cv.hovered(r) {
                        Ink::Text
                    } else {
                        Ink::Dim
                    };
                    cv.text(w.title(), Font::Label, ink, r, Align::Center);
                }

                let (pin, ink) = match (self.settings.pinned, cv.hovered(PIN)) {
                    (true, _) => (glyph::PINNED, Ink::Accent),
                    (false, true) => (glyph::PIN, Ink::Text),
                    (false, false) => (glyph::PIN, Ink::Dim),
                };
                cv.text(pin, Font::Icon, ink, PIN, Align::Center);

                self.widgets[self.active].draw(&cv.offset(CONTENT.x, CONTENT.y), CONTENT.w, CONTENT.h);
                cv.pop_clip();
            }

            match self.rt.EndDraw(None, None) {
                Ok(()) => {}
                Err(err) if err.code() == D2DERR_RECREATE_TARGET => {
                    // Device lost: rebuild the target and brushes, redraw next tick.
                    if let Ok((rt, brushes)) = create_target(&self.d2d) {
                        self.rt = rt;
                        self.brushes = brushes;
                    }
                    self.dirty = true;
                    return;
                }
                Err(_) => return,
            }

            let dst = POINT { x: l.x, y: l.y };
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
        a.menu_open = from_notch;
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

    with_app(|a| a.menu_open = false);
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
            with_app(|a| a.on_click(x, y));
            LRESULT(0)
        }
        WM_RBUTTONUP => {
            show_menu(hwnd, true);
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
    use super::in_notch;

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
