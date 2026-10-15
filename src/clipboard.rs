//! Clipboard capture for the history widget. A hidden message-only window listens for
//! clipboard changes and snapshots text and bitmaps into an in-memory queue; nothing is
//! written to disk. Copies marked private (password managers do this) are skipped, using the
//! same markers Windows' own clipboard history honours.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use windows::{
    Win32::{
        Foundation::{GlobalFree, HANDLE, HGLOBAL, HWND, LPARAM, LRESULT, WPARAM},
        System::{
            DataExchange::{
                AddClipboardFormatListener, CloseClipboard, EmptyClipboard, GetClipboardData,
                IsClipboardFormatAvailable, OpenClipboard, RegisterClipboardFormatW, RemoveClipboardFormatListener,
                SetClipboardData,
            },
            LibraryLoader::GetModuleHandleW,
            Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock},
            Ole::{CF_DIB, CF_UNICODETEXT},
        },
        UI::WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DestroyWindow, HWND_MESSAGE, KillTimer, RegisterClassW, SetTimer,
            WINDOW_EX_STYLE, WINDOW_STYLE, WM_CLIPBOARDUPDATE, WM_TIMER, WNDCLASSW,
        },
    },
    core::{PCWSTR, w},
};

use crate::gfx::Image;
use crate::imaging;

/// Longest text kept (in UTF-16 units); longer copies are truncated.
const MAX_TEXT: usize = 1 << 20;
/// Biggest bitmap kept (bytes); larger ones are skipped.
const MAX_DIB: usize = 64 << 20;
/// Thumbnail size for bitmaps, in pixels.
const THUMB: u32 = 128;
/// Wait after a clipboard change before reading it (see `wndproc`).
const CAPTURE_TIMER: usize = 1;
const SETTLE_MS: u32 = 150;

pub enum Content {
    Text(String),
    Image { thumb: Arc<Image>, width: u32, height: u32, dib: Arc<Vec<u8>> },
}

impl Content {
    pub fn same_as(&self, other: &Content) -> bool {
        match (self, other) {
            (Content::Text(a), Content::Text(b)) => a == b,
            (Content::Image { dib: a, .. }, Content::Image { dib: b, .. }) => a == b,
            _ => false,
        }
    }
}

pub struct Clip {
    /// Stable identity, kept when a repeat copy moves an item back to the top.
    pub id: u64,
    pub content: Content,
    pub at: Instant,
}

thread_local! {
    static CAPTURED: RefCell<VecDeque<Clip>> = const { RefCell::new(VecDeque::new()) };
}

/// Owns the listener window. Must be created and used on the UI thread.
pub struct Watcher {
    hwnd: HWND,
}

impl Watcher {
    pub fn new() -> Option<Self> {
        unsafe {
            let hinst = GetModuleHandleW(None).ok()?;
            let class = w!("MyNotchClipboard");
            let wc = WNDCLASSW { hInstance: hinst.into(), lpszClassName: class, lpfnWndProc: Some(wndproc), ..Default::default() };
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
            .ok()?;
            if AddClipboardFormatListener(hwnd).is_err() {
                let _ = DestroyWindow(hwnd);
                return None;
            }
            Some(Self { hwnd })
        }
    }

    /// Clips captured since the last call, oldest first.
    pub fn drain(&self) -> Vec<Clip> {
        CAPTURED.with(|q| q.borrow_mut().drain(..).collect())
    }

    /// Puts `content` back on the clipboard. Returns whether it worked.
    pub fn copy(&self, content: &Content) -> bool {
        let Some(_open) = OpenGuard::new(self.hwnd) else { return false };
        unsafe {
            if EmptyClipboard().is_err() {
                return false;
            }
            match content {
                Content::Text(s) => set_text(s),
                Content::Image { dib, .. } => set_data(u32::from(CF_DIB.0), dib),
            }
        }
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        unsafe {
            let _ = RemoveClipboardFormatListener(self.hwnd);
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

/// The clipboard's text, or `None` if it has none, it's too big, or the clipboard is busy.
/// Not filtered for private copies: the user is pasting into our own field.
pub fn read_text(owner: HWND) -> Option<String> {
    let _open = OpenGuard::new(owner)?;
    unsafe {
        IsClipboardFormatAvailable(u32::from(CF_UNICODETEXT.0)).ok()?;
        read_global(u32::from(CF_UNICODETEXT.0), MAX_TEXT * 2).map(|bytes| utf16_text(&bytes))
    }
}

/// Puts `text` on the clipboard, owned by `owner` (it must be a real window: with no owner,
/// `SetClipboardData` fails after `EmptyClipboard`). Returns whether it worked. The history
/// widget picks it up like any other copy.
pub fn write_text(owner: HWND, text: &str) -> bool {
    let Some(_open) = OpenGuard::new(owner) else { return false };
    unsafe { EmptyClipboard().is_ok() && set_text(text) }
}

/// Puts `s` on the open clipboard as CF_UNICODETEXT.
unsafe fn set_text(s: &str) -> bool {
    let wide: Vec<u16> = s.encode_utf16().chain([0]).collect();
    unsafe {
        let bytes = std::slice::from_raw_parts(wide.as_ptr().cast::<u8>(), wide.len() * 2);
        set_data(u32::from(CF_UNICODETEXT.0), bytes)
    }
}

/// CF_UNICODETEXT bytes (UTF-16LE) as a string, up to the first NUL.
fn utf16_text(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|&c| u16::from_le_bytes(c)).collect();
    let end = units.iter().position(|&u| u == 0).unwrap_or(units.len());
    String::from_utf16_lossy(&units[..end])
}

/// Copies `bytes` into a movable global block and hands it to the open clipboard.
unsafe fn set_data(format: u32, bytes: &[u8]) -> bool {
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

/// Opens the clipboard (retrying briefly, since another app may hold it) and closes it on drop.
struct OpenGuard;

impl OpenGuard {
    fn new(hwnd: HWND) -> Option<Self> {
        for _ in 0..5 {
            if unsafe { OpenClipboard(Some(hwnd)) }.is_ok() {
                return Some(Self);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        None
    }
}

impl Drop for OpenGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseClipboard();
        }
    }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        // Don't read straight away: many apps (anything using OLE, e.g. .NET and Office)
        // announce the data first and finish publishing it a moment later, and opening the
        // clipboard in between makes their final step fail. Read once things have settled;
        // re-arming the timer also coalesces bursts of updates.
        WM_CLIPBOARDUPDATE => {
            unsafe { SetTimer(Some(hwnd), CAPTURE_TIMER, SETTLE_MS, None) };
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == CAPTURE_TIMER => {
            unsafe {
                let _ = KillTimer(Some(hwnd), CAPTURE_TIMER);
            }
            if let Some(content) = capture(hwnd) {
                static NEXT_ID: AtomicU64 = AtomicU64::new(1);
                let clip = Clip { id: NEXT_ID.fetch_add(1, Ordering::Relaxed), content, at: Instant::now() };
                CAPTURED.with(|q| q.borrow_mut().push_back(clip));
            }
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn capture(hwnd: HWND) -> Option<Content> {
    let _open = OpenGuard::new(hwnd)?;
    unsafe {
        if is_private() {
            return None;
        }
        if IsClipboardFormatAvailable(u32::from(CF_UNICODETEXT.0)).is_ok() {
            let text = utf16_text(&read_global(u32::from(CF_UNICODETEXT.0), MAX_TEXT * 2)?);
            return (!text.trim().is_empty()).then_some(Content::Text(text));
        }
        if IsClipboardFormatAvailable(u32::from(CF_DIB.0)).is_ok() {
            let dib = read_global(u32::from(CF_DIB.0), MAX_DIB)?;
            let (width, height, bgra) = dib_to_bgra(&dib)?;
            let thumb = imaging::from_bgra(width, height, width as usize * 4, &bgra, true, THUMB)?;
            return Some(Content::Image { thumb: Arc::new(thumb), width, height, dib: Arc::new(dib) });
        }
        None
    }
}

/// The markers apps set to keep a copy out of clipboard history and monitors.
unsafe fn is_private() -> bool {
    unsafe {
        let present = |name: PCWSTR| {
            let format = RegisterClipboardFormatW(name);
            format != 0 && IsClipboardFormatAvailable(format).is_ok()
        };
        if present(w!("ExcludeClipboardContentFromMonitorProcessing")) || present(w!("Clipboard Viewer Ignore")) {
            return true;
        }
        let history = RegisterClipboardFormatW(w!("CanIncludeInClipboardHistory"));
        // Present with a zero DWORD means "don't keep this".
        history != 0
            && IsClipboardFormatAvailable(history).is_ok()
            && read_global(history, 4).is_some_and(|b| b.len() >= 4 && b[..4] == [0, 0, 0, 0])
    }
}

/// Copies a clipboard format's global block, refusing ones larger than `max` bytes.
unsafe fn read_global(format: u32, max: usize) -> Option<Vec<u8>> {
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

/// Decodes a packed DIB (BITMAPINFOHEADER + pixels; 24 or 32 bpp, uncompressed or 32 bpp
/// bitfields) into top-down BGRA with straight alpha.
fn dib_to_bgra(dib: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let u32_at = |o: usize| dib.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let i32_at = |o: usize| u32_at(o).map(|v| v as i32);
    let header = u32_at(0)? as usize;
    let (width, height) = (i32_at(4)?, i32_at(8)?);
    let bpp = u16::from_le_bytes([*dib.get(14)?, *dib.get(15)?]);
    let compression = u32_at(16)?;
    if header < 40 || width <= 0 || height == 0 || !matches!((bpp, compression), (24, 0) | (32, 0) | (32, 3)) {
        return None;
    }
    // BI_BITFIELDS with the plain 40-byte header puts three colour masks after it.
    let offset = header + if compression == 3 && header == 40 { 12 } else { 0 };
    let (w, h, top_down) = (width as usize, height.unsigned_abs() as usize, height < 0);
    let stride = (w * bpp as usize).div_ceil(32) * 4;
    let pixels = dib.get(offset..offset.checked_add(stride.checked_mul(h)?)?)?;

    let mut out = vec![0u8; w * h * 4];
    for y in 0..h {
        let row = &pixels[(if top_down { y } else { h - 1 - y }) * stride..];
        for x in 0..w {
            let o = &mut out[(y * w + x) * 4..][..4];
            if bpp == 32 {
                o.copy_from_slice(&row[x * 4..x * 4 + 4]);
            } else {
                o[..3].copy_from_slice(&row[x * 3..x * 3 + 3]);
                o[3] = 255;
            }
        }
    }
    // Many apps leave the alpha byte of 32 bpp DIBs at zero; that means opaque.
    if bpp == 32 && out.as_chunks::<4>().0.iter().all(|p| p[3] == 0) {
        out.as_chunks_mut::<4>().0.iter_mut().for_each(|p| p[3] = 255);
    }
    Some((w as u32, h as u32, out))
}

/// Whether copied text is a single web link.
pub fn is_link(text: &str) -> bool {
    let t = text.trim();
    (t.starts_with("http://") || t.starts_with("https://") || t.starts_with("www.")) && !t.contains(char::is_whitespace)
}

/// One-line preview: whitespace runs collapsed, at most `max` characters.
pub fn preview(text: &str, max: usize) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(max).collect()
}

#[cfg(test)]
mod tests {
    use super::{dib_to_bgra, is_link, preview, utf16_text};

    fn header(width: i32, height: i32, bpp: u16, compression: u32) -> Vec<u8> {
        let mut h = vec![0u8; 40];
        h[0..4].copy_from_slice(&40u32.to_le_bytes());
        h[4..8].copy_from_slice(&width.to_le_bytes());
        h[8..12].copy_from_slice(&height.to_le_bytes());
        h[12..14].copy_from_slice(&1u16.to_le_bytes());
        h[14..16].copy_from_slice(&bpp.to_le_bytes());
        h[16..20].copy_from_slice(&compression.to_le_bytes());
        h
    }

    #[test]
    fn decodes_bottom_up_24bpp_with_row_padding() {
        // 2x2, rows padded from 6 to 8 bytes; bottom row first.
        let mut dib = header(2, 2, 24, 0);
        dib.extend([1, 2, 3, 4, 5, 6, 0, 0]); // bottom row
        dib.extend([7, 8, 9, 10, 11, 12, 0, 0]); // top row
        let (w, h, px) = dib_to_bgra(&dib).unwrap();
        assert_eq!((w, h), (2, 2));
        assert_eq!(&px[..8], &[7, 8, 9, 255, 10, 11, 12, 255], "top row first");
        assert_eq!(&px[8..], &[1, 2, 3, 255, 4, 5, 6, 255]);
    }

    #[test]
    fn zero_alpha_32bpp_means_opaque_and_bitfields_masks_are_skipped() {
        let mut dib = header(1, -1, 32, 3);
        dib.extend([0; 12]); // colour masks
        dib.extend([10, 20, 30, 0]);
        assert_eq!(dib_to_bgra(&dib).unwrap().2, vec![10, 20, 30, 255]);
    }

    #[test]
    fn rejects_truncated_and_unsupported() {
        let mut dib = header(4, 4, 32, 0);
        dib.extend([0; 10]);
        assert!(dib_to_bgra(&dib).is_none(), "too short");
        assert!(dib_to_bgra(&header(4, 4, 8, 0)).is_none(), "paletted");
    }

    #[test]
    fn links_and_previews() {
        assert!(is_link("https://github.com/ContactEstablished/mynotch"));
        assert!(is_link("  www.example.com  "));
        assert!(!is_link("see https://example.com"));
        assert_eq!(preview("  fn main() {\n    println!(\"hi\");\n}  ", 100), "fn main() { println!(\"hi\"); }");
        assert_eq!(preview("abcdef", 3), "abc");
    }

    #[test]
    fn utf16_text_stops_at_the_first_nul() {
        let bytes = |s: &str| s.encode_utf16().flat_map(u16::to_le_bytes).collect::<Vec<u8>>();
        assert_eq!(utf16_text(&bytes("hi\0junk")), "hi");
        assert_eq!(utf16_text(&bytes("héllo 😀")), "héllo 😀", "no NUL: the whole buffer");
    }
}
