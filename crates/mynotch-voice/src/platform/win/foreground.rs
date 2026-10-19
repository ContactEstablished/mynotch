//! The window the user is working in, so a session can say where its text was meant to go.

use std::ffi::c_void;

use windows::Win32::Foundation::{CloseHandle, HWND};
use windows::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
use windows::core::{HSTRING, PWSTR, w};

/// The foreground window when it was looked at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Foreground {
    /// The window handle, as a number (comparable, and `Send` unlike `HWND`).
    pub hwnd: usize,
    pub pid: u32,
    /// "notepad.exe", or empty if the process couldn't be asked (e.g. it runs elevated).
    pub exe: String,
    /// What the app calls itself ("Notepad", "Google Chrome"), or the exe's name without
    /// ".exe"; empty if unknown.
    pub app: String,
}

pub fn foreground() -> Foreground {
    let hwnd = unsafe { GetForegroundWindow() };
    let mut pid = 0;
    if !hwnd.is_invalid() {
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    }
    let path = exe_path(pid);
    let exe = path.rsplit('\\').next().unwrap_or_default().to_owned();
    let app = description(&path).unwrap_or_else(|| exe.trim_end_matches(".exe").trim_end_matches(".EXE").to_owned());
    Foreground { hwnd: hwnd_id(hwnd), pid, exe, app }
}

fn hwnd_id(hwnd: HWND) -> usize {
    hwnd.0 as usize
}

fn exe_path(pid: u32) -> String {
    if pid == 0 {
        return String::new();
    }
    unsafe {
        let Ok(process) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return String::new() };
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
        let _ = CloseHandle(process);
        if ok { String::from_utf16_lossy(&buf[..len as usize]) } else { String::new() }
    }
}

/// The exe's `FileDescription` (in its first listed language), if it has a useful one.
fn description(path: &str) -> Option<String> {
    if path.is_empty() {
        return None;
    }
    let path = HSTRING::from(path);
    unsafe {
        let size = GetFileVersionInfoSizeW(&path, None);
        if size == 0 {
            return None;
        }
        let mut data = vec![0u8; size as usize];
        GetFileVersionInfoW(&path, None, size, data.as_mut_ptr().cast()).ok()?;
        let block = data.as_ptr().cast::<c_void>();
        let (mut ptr, mut len) = (std::ptr::null_mut::<c_void>(), 0u32);
        if !VerQueryValueW(block, w!("\\VarFileInfo\\Translation"), &mut ptr, &mut len).as_bool() || len < 4 {
            return None;
        }
        let [language, codepage] = ptr.cast::<[u16; 2]>().read_unaligned();
        let key = HSTRING::from(format!("\\StringFileInfo\\{language:04x}{codepage:04x}\\FileDescription"));
        if !VerQueryValueW(block, &key, &mut ptr, &mut len).as_bool() || len == 0 {
            return None;
        }
        let text = std::slice::from_raw_parts(ptr.cast::<u16>(), len as usize);
        let end = text.iter().position(|&u| u == 0).unwrap_or(text.len());
        let name = String::from_utf16_lossy(&text[..end]).trim().to_owned();
        // Some exes describe themselves by file name, or not at all.
        (!name.is_empty() && !name.to_ascii_lowercase().ends_with(".exe")).then_some(name)
    }
}
