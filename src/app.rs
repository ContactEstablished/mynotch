use std::cell::RefCell;
use std::sync::atomic::{AtomicU32, Ordering};

use windows::{
    Win32::{
        Foundation::*,
        Graphics::{
            Direct2D::{Common::*, *},
            DirectWrite::*,
            Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM,
            Gdi::*,
        },
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            HiDpi::*,
            Shell::{NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW, Shell_NotifyIconW},
            WindowsAndMessaging::*,
        },
    },
    core::*,
};
use windows_numerics::Vector2;

use crate::stats::{Snapshot, Stats};

const TIMER_FRAME: usize = 1;
const TIMER_STATS: usize = 2;

const WM_TRAY: u32 = WM_APP + 1;
const TRAY_ID: u32 = 1;
const CMD_PIN: usize = 1;
const CMD_QUIT: usize = 2;

// Logical (96-DPI) sizes.
const COLLAPSED: (f32, f32) = (190.0, 32.0);
const EXPANDED: (f32, f32) = (520.0, 150.0);
const RADIUS: f32 = 20.0;
/// Slack around the notch so the cursor grazing its edge doesn't flicker it.
const HOVER_MARGIN: f32 = 6.0;

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
    /// Window origin and size (the expanded footprint), in physical screen pixels.
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    surface: Surface,
    fmt_small: IDWriteTextFormat,
    fmt_big: IDWriteTextFormat,
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

            let w = (EXPANDED.0 * scale).ceil() as i32;
            let h = (EXPANDED.1 * scale).ceil() as i32 + 1;
            let m = info.rcMonitor;

            let make_fmt = |size: f32, weight: DWRITE_FONT_WEIGHT| {
                dwrite.CreateTextFormat(
                    w!("Segoe UI"),
                    None,
                    weight,
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    size * scale,
                    w!("en-us"),
                )
            };

            Ok(Self {
                scale,
                x: m.left + (m.right - m.left - w) / 2,
                y: m.top,
                w,
                h,
                surface: Surface::new(w, h)?,
                fmt_small: make_fmt(12.0, DWRITE_FONT_WEIGHT_NORMAL)?,
                fmt_big: make_fmt(22.0, DWRITE_FONT_WEIGHT_SEMI_BOLD)?,
            })
        }
    }
}

/// Device-dependent brushes; rebuilt together with the render target.
struct Brushes {
    black: ID2D1SolidColorBrush,
    white: ID2D1SolidColorBrush,
    dim: ID2D1SolidColorBrush,
    track: ID2D1SolidColorBrush,
    accent: ID2D1SolidColorBrush,
}

impl Brushes {
    unsafe fn new(rt: &ID2D1DCRenderTarget) -> Result<Self> {
        let solid = |r: f32, g: f32, b: f32| unsafe {
            rt.CreateSolidColorBrush(&D2D1_COLOR_F { r, g, b, a: 1.0 }, None)
        };
        Ok(Self {
            black: solid(0.0, 0.0, 0.0)?,
            white: solid(0.95, 0.95, 0.95)?,
            dim: solid(0.6, 0.6, 0.65)?,
            track: solid(0.2, 0.2, 0.24)?,
            accent: solid(0.35, 0.65, 1.0)?,
        })
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
    stats: Stats,
    snap: Snapshot,
    /// 0.0 = collapsed, 1.0 = expanded.
    t: f32,
    pinned: bool,
    /// Keeps the notch open while its own context menu is showing.
    menu_open: bool,
    dirty: bool,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

pub fn run() -> Result<()> {
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
        app.sample_stats();
        app.render();
        APP.with(|a| *a.borrow_mut() = Some(app));

        tray_add(hwnd);
        SetTimer(Some(hwnd), TIMER_FRAME, 16, None);
        SetTimer(Some(hwnd), TIMER_STATS, 1000, None);
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

            Ok(Self {
                hwnd,
                d2d,
                dwrite,
                rt,
                brushes,
                layout,
                stats: Stats::new(),
                snap: Snapshot::default(),
                t: 0.0,
                pinned: false,
                menu_open: false,
                dirty: true,
            })
        }
    }

    fn sample_stats(&mut self) {
        let s = self.stats.sample();
        if s != self.snap {
            self.snap = s;
            self.dirty = true;
        }
    }

    /// Re-fit to the primary monitor after a resolution, DPI or arrangement change.
    fn relayout(&mut self) {
        if let Ok(layout) = unsafe { Layout::primary(&self.dwrite) } {
            self.layout = layout;
            self.render();
        }
    }

    fn toggle_pin(&mut self) {
        self.pinned = !self.pinned;
        self.dirty = true;
    }

    /// Current notch size in physical pixels.
    fn notch_size(&self) -> (f32, f32) {
        let e = ease(self.t);
        let w = lerp(COLLAPSED.0, EXPANDED.0, e) * self.layout.scale;
        let h = lerp(COLLAPSED.1, EXPANDED.1, e) * self.layout.scale;
        (w, h)
    }

    /// Per-frame: hover detection and animation.
    fn tick(&mut self) {
        let mut pt = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut pt);
        }
        let l = &self.layout;
        let (nw, nh) = self.notch_size();
        let m = HOVER_MARGIN * l.scale;
        let left = l.x as f32 + (l.w as f32 - nw) / 2.0 - m;
        let (px, py) = (pt.x as f32, pt.y as f32);
        let hovered = px >= left
            && px < left + nw + m * 2.0
            && py >= l.y as f32
            && py < l.y as f32 + nh + m;

        let target = if hovered || self.pinned || self.menu_open { 1.0 } else { 0.0 };
        if (self.t - target).abs() > 0.001 {
            self.t += (target - self.t) * 0.22;
            if (self.t - target).abs() < 0.005 {
                self.t = target;
            }
            self.dirty = true;
        }
        if self.dirty {
            self.render();
        }
    }

    fn pin_rect(&self) -> (f32, f32, f32) {
        // Centre x, centre y, radius of the pin button (physical px, window-relative).
        let s = self.layout.scale;
        let (nw, _) = self.notch_size();
        let right = (self.layout.w as f32 + nw) / 2.0;
        (right - 26.0 * s, 24.0 * s, 6.0 * s)
    }

    fn on_click(&mut self, x: i32, y: i32) {
        if self.t < 0.9 {
            return;
        }
        let (cx, cy, r) = self.pin_rect();
        let (dx, dy) = (x as f32 - cx, y as f32 - cy);
        if dx * dx + dy * dy <= (r * 2.0) * (r * 2.0) {
            self.toggle_pin();
        }
    }

    fn render(&mut self) {
        self.dirty = false;
        let l = &self.layout;
        let b = &self.brushes;
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

            // Body: extend above the window so the top corners stay flat.
            let r = RADIUS * s;
            let body = D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F { left, top: -r, right: left + nw, bottom: nh },
                radiusX: r,
                radiusY: r,
            };
            self.rt.FillRoundedRectangle(&body, &b.black);

            // Collapsed content: tiny live readout.
            if e < 0.5 {
                b.dim.SetOpacity(1.0 - e * 2.0);
                let text = format!("CPU {:.0}%  RAM {:.0}%", self.snap.cpu, self.snap.ram);
                self.text(&text, &l.fmt_small, left, 0.0, nw, nh, &b.dim, true);
            }

            // Expanded content.
            if e > 0.5 {
                let a = (e - 0.5) * 2.0;
                for brush in [&b.white, &b.dim, &b.track, &b.accent] {
                    brush.SetOpacity(a);
                }

                let pad = 28.0 * s;
                let x0 = left + pad;
                let bar_w = nw - pad * 2.0;
                self.text("System", &l.fmt_big, x0, 22.0 * s, 200.0 * s, 30.0 * s, &b.white, false);

                let rows = [("CPU", self.snap.cpu), ("RAM", self.snap.ram)];
                for (i, (label, pct)) in rows.iter().enumerate() {
                    let y = (66.0 + i as f32 * 36.0) * s;
                    self.text(label, &l.fmt_small, x0, y, 50.0 * s, 18.0 * s, &b.dim, false);
                    let bx = x0 + 50.0 * s;
                    let bw = bar_w - 100.0 * s;
                    let bar = |w: f32| D2D1_ROUNDED_RECT {
                        rect: D2D_RECT_F {
                            left: bx,
                            top: y + 5.0 * s,
                            right: bx + w,
                            bottom: y + 13.0 * s,
                        },
                        radiusX: 4.0 * s,
                        radiusY: 4.0 * s,
                    };
                    self.rt.FillRoundedRectangle(&bar(bw), &b.track);
                    self.rt.FillRoundedRectangle(&bar((bw * pct / 100.0).max(1.0)), &b.accent);
                    let v = format!("{pct:.0}%");
                    self.text(&v, &l.fmt_small, bx + bw + 10.0 * s, y, 40.0 * s, 18.0 * s, &b.white, false);
                }

                // Pin button.
                let (cx, cy, pr) = self.pin_rect();
                let c = D2D1_ELLIPSE { point: Vector2 { X: cx, Y: cy }, radiusX: pr, radiusY: pr };
                if self.pinned {
                    self.rt.FillEllipse(&c, &b.accent);
                } else {
                    self.rt.DrawEllipse(&c, &b.dim, 1.5 * s, None);
                }
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
                SourceConstantAlpha: 255,
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

    #[allow(clippy::too_many_arguments)]
    unsafe fn text(
        &self,
        s: &str,
        fmt: &IDWriteTextFormat,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        brush: &ID2D1SolidColorBrush,
        center: bool,
    ) {
        unsafe {
            let wide: Vec<u16> = s.encode_utf16().collect();
            let _ = fmt.SetTextAlignment(if center {
                DWRITE_TEXT_ALIGNMENT_CENTER
            } else {
                DWRITE_TEXT_ALIGNMENT_LEADING
            });
            let _ = fmt.SetParagraphAlignment(if center {
                DWRITE_PARAGRAPH_ALIGNMENT_CENTER
            } else {
                DWRITE_PARAGRAPH_ALIGNMENT_NEAR
            });
            let rect = D2D_RECT_F { left: x, top: y, right: x + w, bottom: y + h };
            self.rt.DrawText(
                &wide,
                fmt,
                &rect,
                brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        }
    }
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
        if let Ok(mut guard) = a.try_borrow_mut() {
            if let Some(app) = guard.as_mut() {
                f(app);
            }
        }
    });
}

fn tray_data(hwnd: HWND) -> NOTIFYICONDATAW {
    let mut nid = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: TRAY_ID,
        uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
        uCallbackMessage: WM_TRAY,
        hIcon: unsafe { LoadIconW(None, IDI_APPLICATION) }.unwrap_or_default(),
        ..Default::default()
    };
    for (dst, src) in nid.szTip.iter_mut().zip("MyNotch".encode_utf16()) {
        *dst = src;
    }
    nid
}

fn tray_add(hwnd: HWND) {
    unsafe {
        let _ = Shell_NotifyIconW(NIM_ADD, &tray_data(hwnd));
    }
}

fn tray_remove(hwnd: HWND) {
    unsafe {
        let _ = Shell_NotifyIconW(NIM_DELETE, &tray_data(hwnd));
    }
}

/// Shows the Pin/Quit menu at the cursor. `TrackPopupMenu` runs a modal loop, so this must
/// be called without the `APP` borrow held, letting the frame timer keep animating.
fn show_menu(hwnd: HWND, from_notch: bool) {
    let mut pinned = false;
    with_app(|a| {
        pinned = a.pinned;
        a.menu_open = from_notch;
    });

    let cmd = unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        let pin_flags = if pinned { MF_STRING | MF_CHECKED } else { MF_STRING };
        let _ = AppendMenuW(menu, pin_flags, CMD_PIN, w!("Pin open"));
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
        CMD_QUIT => unsafe {
            let _ = DestroyWindow(hwnd);
        },
        _ => {}
    }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_TIMER => {
            match wparam.0 {
                TIMER_FRAME => with_app(|a| a.tick()),
                TIMER_STATS => with_app(|a| a.sample_stats()),
                _ => {}
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
        WM_DESTROY => {
            tray_remove(hwnd);
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ if msg != 0 && msg == TASKBAR_CREATED.load(Ordering::Relaxed) => {
            tray_add(hwnd);
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}
