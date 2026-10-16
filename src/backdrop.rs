//! The blur behind the glass. The notch is a layered window that paints its own pixels, so
//! DWM can't blur behind its rounded shape; instead a second, click-through window sits just
//! below it, inside the body, and asks DWM to blur whatever is underneath. The notch then
//! paints its tinted glass over that.

use std::ffi::c_void;

use windows::{
    Win32::{
        Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM},
        Graphics::{
            Dwm::{DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute},
            Gdi::{BLACK_BRUSH, GetStockObject, HBRUSH},
        },
        System::LibraryLoader::{GetModuleHandleW, GetProcAddress},
        UI::WindowsAndMessaging::*,
    },
    core::{BOOL, s, w},
};

/// `SetWindowCompositionAttribute`'s accent policy: undocumented, but stable since Windows 10
/// and what the shell itself uses for blur.
#[repr(C)]
struct AccentPolicy {
    state: u32,
    flags: u32,
    gradient: u32,
    animation: u32,
}

#[repr(C)]
struct CompositionData {
    attribute: u32,
    data: *mut c_void,
    size: usize,
}

const WCA_ACCENT_POLICY: u32 = 19;
const ACCENT_ENABLE_BLURBEHIND: u32 = 3;

type SetWindowCompositionAttribute = unsafe extern "system" fn(HWND, *mut CompositionData) -> BOOL;

/// Asks DWM to blur behind `hwnd`. False where the call isn't available or is refused.
fn enable_blur(hwnd: HWND) -> bool {
    unsafe {
        let Ok(user32) = GetModuleHandleW(w!("user32.dll")) else { return false };
        let Some(proc) = GetProcAddress(user32, s!("SetWindowCompositionAttribute")) else { return false };
        let set: SetWindowCompositionAttribute = std::mem::transmute(proc);
        let mut accent = AccentPolicy { state: ACCENT_ENABLE_BLURBEHIND, flags: 0, gradient: 0, animation: 0 };
        let mut data = CompositionData {
            attribute: WCA_ACCENT_POLICY,
            data: (&raw mut accent).cast(),
            size: size_of::<AccentPolicy>(),
        };
        set(hwnd, &mut data).as_bool()
    }
}

/// DWM rounds the blur's corners by this much (logical px; Windows 11's standard radius).
pub const CORNER: f32 = 8.0;

pub struct Backdrop {
    hwnd: HWND,
    /// Where it is now, in screen px; `None` while hidden.
    at: Option<RECT>,
}

impl Backdrop {
    /// `None` if the blur isn't available (then the glass is simply tinted).
    pub fn new(hinst: HINSTANCE) -> Option<Self> {
        unsafe {
            let class = w!("MyNotchBackdrop");
            let wc = WNDCLASSW {
                hInstance: hinst,
                lpszClassName: class,
                lpfnWndProc: Some(wndproc),
                // Black is see-through here: the blur shows instead.
                hbrBackground: HBRUSH(GetStockObject(BLACK_BRUSH).0),
                ..Default::default()
            };
            RegisterClassW(&wc);
            // Layered and transparent, so every click goes straight through it.
            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                class,
                w!("MyNotch backdrop"),
                WS_POPUP,
                0,
                0,
                0,
                0,
                None,
                None,
                Some(hinst),
                None,
            )
            .ok()?;
            let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA);
            // Rounded corners clip the blur (a window region doesn't), and no border.
            let round = DWMWCP_ROUND;
            let _ = DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, (&raw const round).cast(), 4);
            let none = DWMWA_COLOR_NONE;
            let _ = DwmSetWindowAttribute(hwnd, DWMWA_BORDER_COLOR, (&raw const none).cast(), 4);
            if !enable_blur(hwnd) {
                let _ = DestroyWindow(hwnd);
                return None;
            }
            Some(Self { hwnd, at: None })
        }
    }

    /// Blurs `r` (screen px), just below `above` in the z-order; `None` hides the blur.
    pub fn place(&mut self, above: HWND, r: Option<RECT>) {
        if r == self.at {
            return;
        }
        self.at = r;
        unsafe {
            match r {
                Some(r) => {
                    let flags = SWP_NOACTIVATE | SWP_SHOWWINDOW | SWP_NOOWNERZORDER;
                    let _ = SetWindowPos(self.hwnd, Some(above), r.left, r.top, r.right - r.left, r.bottom - r.top, flags);
                }
                None => {
                    let _ = ShowWindow(self.hwnd, SW_HIDE);
                }
            }
        }
    }
}

impl Drop for Backdrop {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        // Never take the focus, even if something tries.
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}
