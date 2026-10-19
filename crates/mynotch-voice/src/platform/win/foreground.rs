//! The window the user is working in, so a session can say where its text was meant to go.

use windows::Win32::Foundation::{CloseHandle, HWND};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
use windows::core::PWSTR;

/// The foreground window when it was looked at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Foreground {
    /// The window handle, as a number (comparable, and `Send` unlike `HWND`).
    pub hwnd: usize,
    pub pid: u32,
    /// "notepad.exe", or empty if the process couldn't be asked (e.g. it runs elevated).
    pub exe: String,
}

pub fn foreground() -> Foreground {
    let hwnd = unsafe { GetForegroundWindow() };
    let mut pid = 0;
    if !hwnd.is_invalid() {
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    }
    Foreground { hwnd: hwnd_id(hwnd), pid, exe: exe_name(pid) }
}

fn hwnd_id(hwnd: HWND) -> usize {
    hwnd.0 as usize
}

fn exe_name(pid: u32) -> String {
    if pid == 0 {
        return String::new();
    }
    unsafe {
        let Ok(process) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return String::new() };
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
        let _ = CloseHandle(process);
        if !ok {
            return String::new();
        }
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        path.rsplit('\\').next().unwrap_or_default().to_owned()
    }
}
