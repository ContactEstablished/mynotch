//! Integration with the Windows desktop: single-instance guard, Start with Windows, and
//! fullscreen detection.

use windows::{
    Win32::{
        Foundation::{ERROR_ALREADY_EXISTS, ERROR_SUCCESS, GetLastError, RECT},
        System::{
            Registry::{
                HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
            },
            Threading::CreateMutexW,
        },
        UI::WindowsAndMessaging::{GetClassNameW, GetForegroundWindow, GetWindowRect, GetWindowThreadProcessId, IsZoomed},
    },
    core::{PCWSTR, w},
};

/// Returns false if another MyNotch is already running in this session. The mutex is never
/// closed, so it is held for the life of the process.
pub fn claim_single_instance() -> bool {
    unsafe {
        match CreateMutexW(None, false, w!("Local\\MyNotch.SingleInstance")) {
            Ok(_) => GetLastError() != ERROR_ALREADY_EXISTS,
            // Can't tell; better to risk a second notch than to refuse to start.
            Err(_) => true,
        }
    }
}

/// Start with Windows, via the current user's `Run` key.
pub mod autostart {
    use super::*;

    const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
    const VALUE: PCWSTR = w!("MyNotch");

    pub fn enabled() -> bool {
        unsafe { RegGetValueW(HKEY_CURRENT_USER, RUN_KEY, VALUE, RRF_RT_REG_SZ, None, None, None) == ERROR_SUCCESS }
    }

    /// Registers this executable to launch at sign-in, or removes the registration.
    /// Returns whether it succeeded.
    pub fn set(on: bool) -> bool {
        unsafe {
            if !on {
                return RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, VALUE) == ERROR_SUCCESS;
            }
            let Ok(exe) = std::env::current_exe() else { return false };
            let command: Vec<u16> = format!("\"{}\"", exe.display()).encode_utf16().chain([0]).collect();
            let bytes = (command.len() * size_of::<u16>()) as u32;
            RegSetKeyValueW(HKEY_CURRENT_USER, RUN_KEY, VALUE, REG_SZ.0, Some(command.as_ptr().cast()), bytes)
                == ERROR_SUCCESS
        }
    }
}

/// Whether the taskbar uses the light theme (Settings > Personalization > Colors).
pub fn taskbar_is_light() -> bool {
    let (mut value, mut size) = (0u32, size_of::<u32>() as u32);
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("SystemUsesLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some((&raw mut value).cast()),
            Some(&mut size),
        ) == ERROR_SUCCESS
            && value != 0
    }
}

/// Whether the foreground window covers all of `monitor`: a fullscreen game, video or
/// presentation. Our own windows don't count.
pub fn foreground_is_fullscreen(monitor: RECT) -> bool {
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_invalid() {
            return false;
        }
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == std::process::id() {
            return false;
        }
        let mut class = [0u16; 64];
        let len = GetClassNameW(hwnd, &mut class).max(0) as usize;
        let mut rect = RECT::default();
        if GetWindowRect(hwnd, &mut rect).is_err() {
            return false;
        }
        covers_monitor(&String::from_utf16_lossy(&class[..len]), IsZoomed(hwnd).as_bool(), rect, monitor)
    }
}

fn covers_monitor(class: &str, maximized: bool, window: RECT, monitor: RECT) -> bool {
    // The desktop (wallpaper and icons) is a full-monitor window too. Maximized windows
    // count as normal ones: with an auto-hide taskbar they also cover the whole monitor.
    if matches!(class, "Progman" | "WorkerW") || maximized {
        return false;
    }
    window.left <= monitor.left
        && window.top <= monitor.top
        && window.right >= monitor.right
        && window.bottom >= monitor.bottom
}

#[cfg(test)]
mod tests {
    use super::covers_monitor;
    use windows::Win32::Foundation::RECT;

    const MONITOR: RECT = RECT { left: 0, top: 0, right: 5120, bottom: 1440 };

    #[test]
    fn fullscreen_detection() {
        assert!(covers_monitor("Chrome_WidgetWin_1", false, MONITOR, MONITOR), "exact fit");
        assert!(covers_monitor("UnityWndClass", false, RECT { left: -1, top: -1, right: 5121, bottom: 1441 }, MONITOR));
        assert!(!covers_monitor("Chrome_WidgetWin_1", true, RECT { left: -8, top: -8, right: 5128, bottom: 1448 }, MONITOR),
            "maximized with auto-hide taskbar");
        assert!(!covers_monitor("Notepad", false, RECT { left: 0, top: 0, right: 5120, bottom: 1392 }, MONITOR),
            "stops at the taskbar");
        assert!(!covers_monitor("WorkerW", false, MONITOR, MONITOR), "desktop");
        assert!(!covers_monitor("Progman", false, MONITOR, MONITOR), "desktop");
        let second = RECT { left: 5120, top: 0, right: 7680, bottom: 1440 };
        assert!(!covers_monitor("Chrome_WidgetWin_1", false, second, MONITOR), "fullscreen on another monitor");
    }
}
