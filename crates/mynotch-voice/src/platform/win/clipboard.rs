//! The clipboard, for paste delivery: save what the user had, put the dictation there marked
//! private, then put the user's back.

use std::time::Duration;

use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData, GetClipboardSequenceNumber, OpenClipboard,
    RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock};
use windows::Win32::System::Ole::{
    CF_BITMAP, CF_DSPBITMAP, CF_DSPENHMETAFILE, CF_DSPMETAFILEPICT, CF_ENHMETAFILE, CF_GDIOBJFIRST, CF_GDIOBJLAST,
    CF_METAFILEPICT, CF_OWNERDISPLAY, CF_PALETTE, CF_PRIVATEFIRST, CF_PRIVATELAST, CF_UNICODETEXT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, HWND_MESSAGE, RegisterClassW, WINDOW_EX_STYLE, WINDOW_STYLE,
    WNDCLASSW,
};
use windows::core::{PCWSTR, w};

use super::pump_for;

/// The most a snapshot keeps; formats past it are left out.
const MAX_SNAPSHOT: usize = 64 << 20;

/// What was on the clipboard: each format's bytes, in the order the owner offered them (apps
/// paste the first format they understand, so the order matters).
pub struct Snapshot {
    formats: Vec<(u32, Vec<u8>)>,
    /// Formats that couldn't be kept: GDI handles (CF_BITMAP, which Windows makes again from
    /// the DIB formats, but also metafiles, which are lost), private ones, or past
    /// `MAX_SNAPSHOT`.
    pub skipped: usize,
}

impl Snapshot {
    pub fn formats(&self) -> usize {
        self.formats.len()
    }

    pub fn bytes(&self) -> usize {
        self.formats.iter().map(|(_, b)| b.len()).sum()
    }
}

/// A hidden window to own the clipboard while a paste is delivered. The clipboard needs an
/// owner window for `SetClipboardData`, and an owner must answer messages (another app emptying
/// the clipboard sends it one and waits), so waits on this thread go through `pump_for`.
/// Destroying it leaves its data on the clipboard, ownerless, so nothing waits on it after.
pub struct Owner {
    hwnd: HWND,
}

impl Owner {
    pub fn new() -> Result<Self, String> {
        unsafe {
            let hinst = GetModuleHandleW(None).map_err(|e| e.message())?;
            let class = w!("MyNotchVoiceClipboard");
            let wc = WNDCLASSW { hInstance: hinst.into(), lpszClassName: class, lpfnWndProc: Some(wndproc), ..Default::default() };
            // Fails harmlessly once registered.
            RegisterClassW(&wc);
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                class,
                PCWSTR::null(),
                WINDOW_STYLE::default(),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                Some(hinst.into()),
                None,
            )
            .map_err(|e| e.message())?;
            Ok(Self { hwnd })
        }
    }

    pub fn snapshot(&self) -> Result<Snapshot, String> {
        let _open = Open::new(self.hwnd)?;
        let (mut formats, mut skipped, mut total) = (Vec::new(), 0, 0);
        let mut format = 0;
        loop {
            format = unsafe { EnumClipboardFormats(format) };
            if format == 0 {
                break;
            }
            match restorable(format).then(|| unsafe { read(format, MAX_SNAPSHOT - total) }).flatten() {
                Some(bytes) => {
                    total += bytes.len();
                    formats.push((format, bytes));
                }
                None => skipped += 1,
            }
        }
        Ok(Snapshot { formats, skipped })
    }

    /// Puts `text` on the clipboard with the markers that keep it out of clipboard history
    /// (Windows' own, MyNotch's and other managers') and off the cloud clipboard.
    pub fn set_private_text(&self, text: &str) -> Result<(), String> {
        let _open = Open::new(self.hwnd)?;
        let wide: Vec<u8> = text.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect();
        let no = 0u32.to_le_bytes();
        unsafe {
            EmptyClipboard().map_err(|e| e.message())?;
            let ok = write(u32::from(CF_UNICODETEXT.0), &wide)
                && write(RegisterClipboardFormatW(w!("ExcludeClipboardContentFromMonitorProcessing")), &no)
                && write(RegisterClipboardFormatW(w!("CanIncludeInClipboardHistory")), &no)
                && write(RegisterClipboardFormatW(w!("CanUploadToCloudClipboard")), &no);
            if ok { Ok(()) } else { Err("Couldn't set the clipboard".into()) }
        }
    }

    /// Puts a snapshot back (an empty one leaves the clipboard empty). Returns the clipboard's
    /// sequence number afterwards, which identifies this change. It's marked to stay out of
    /// clipboard history and the cloud clipboard: it isn't a new copy, and they have it already.
    pub fn restore(&self, snapshot: &Snapshot) -> Result<u32, String> {
        {
            let _open = Open::new(self.hwnd)?;
            unsafe {
                EmptyClipboard().map_err(|e| e.message())?;
                for (format, bytes) in &snapshot.formats {
                    write(*format, bytes);
                }
                if !snapshot.formats.is_empty() {
                    let no = 0u32.to_le_bytes();
                    for marker in [w!("CanIncludeInClipboardHistory"), w!("CanUploadToCloudClipboard")] {
                        let format = RegisterClipboardFormatW(marker);
                        if !snapshot.formats.iter().any(|(f, _)| *f == format) {
                            write(format, &no);
                        }
                    }
                }
            }
        }
        Ok(sequence())
    }
}

impl Drop for Owner {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// Formats whose data is a plain memory block. The others hold GDI or private handles that
/// can't be copied as bytes.
fn restorable(format: u32) -> bool {
    let not = [CF_BITMAP, CF_METAFILEPICT, CF_PALETTE, CF_ENHMETAFILE, CF_OWNERDISPLAY, CF_DSPBITMAP, CF_DSPMETAFILEPICT, CF_DSPENHMETAFILE];
    let range = |a: u16, b: u16| (u32::from(a)..=u32::from(b)).contains(&format);
    !not.iter().any(|f| u32::from(f.0) == format)
        && !range(CF_PRIVATEFIRST.0, CF_PRIVATELAST.0)
        && !range(CF_GDIOBJFIRST.0, CF_GDIOBJLAST.0)
}

/// The clipboard's change counter (`GetClipboardSequenceNumber`).
pub fn sequence() -> u32 {
    unsafe { GetClipboardSequenceNumber() }
}

/// Copies a format's memory block off the open clipboard, unless it's bigger than `max`.
unsafe fn read(format: u32, max: usize) -> Option<Vec<u8>> {
    unsafe {
        let mem = HGLOBAL(GetClipboardData(format).ok()?.0);
        let size = GlobalSize(mem);
        if size == 0 || size > max {
            return None;
        }
        let src = GlobalLock(mem);
        if src.is_null() {
            return None;
        }
        let bytes = std::slice::from_raw_parts(src.cast::<u8>(), size).to_vec();
        let _ = GlobalUnlock(mem);
        Some(bytes)
    }
}

/// Hands a copy of `bytes` to the open clipboard as `format`.
unsafe fn write(format: u32, bytes: &[u8]) -> bool {
    unsafe {
        let Ok(mem) = GlobalAlloc(GMEM_MOVEABLE, bytes.len()) else { return false };
        let dst = GlobalLock(mem);
        if dst.is_null() {
            let _ = GlobalFree(Some(mem));
            return false;
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), dst.cast::<u8>(), bytes.len());
        let _ = GlobalUnlock(mem);
        if SetClipboardData(format, Some(HANDLE(mem.0))).is_err() {
            // Ownership only passes to the clipboard on success.
            let _ = GlobalFree(Some(mem));
            return false;
        }
        true
    }
}

/// The clipboard, opened (retrying for a while, since another app may hold it) and closed on drop.
struct Open;

impl Open {
    fn new(hwnd: HWND) -> Result<Self, String> {
        for _ in 0..20 {
            if unsafe { OpenClipboard(Some(hwnd)) }.is_ok() {
                return Ok(Self);
            }
            pump_for(Duration::from_millis(10));
        }
        Err("The clipboard is busy".into())
    }
}

impl Drop for Open {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseClipboard();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::restorable;

    #[test]
    fn only_memory_formats_are_restorable() {
        for f in [1, 7, 8, 13, 15, 16, 17, 0x81, 0xC0F0] {
            assert!(restorable(f), "{f}");
        }
        for f in [2, 3, 9, 14, 0x80, 0x82, 0x83, 0x8E, 0x200, 0x2FF, 0x300, 0x3FF] {
            assert!(!restorable(f), "{f}");
        }
    }
}
