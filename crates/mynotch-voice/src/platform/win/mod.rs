//! All of MyNotch Voice's Win32 code, and its other `unsafe` FFI: nothing outside this module
//! calls Windows or uses `unsafe` (plan §0 rule 5).

pub mod clipboard;
pub mod foreground;
pub mod gpu;
pub mod hook;
pub mod input;

use std::time::{Duration, Instant};

use windows::Win32::Foundation::FreeLibrary;
use windows::Win32::System::LibraryLoader::{LOAD_LIBRARY_FLAGS, LoadLibraryExW};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, MSG, MsgWaitForMultipleObjects, PM_REMOVE, PeekMessageW, QS_ALLINPUT, TranslateMessage,
};
use windows::core::HSTRING;

/// What whisper.cpp's CUDA backend needs at run time (CUDA 13): the driver, which the static
/// CUDA runtime loads, and cuBLAS with the cuBLASLt it depends on. The app delay-loads cuBLAS
/// (see its `build.rs`) so it starts without it; voice checks for all three before touching
/// whisper.cpp.
pub const CUDA_DLLS: [&str; 3] = ["nvcuda.dll", "cublas64_13.dll", "cublasLt64_13.dll"];

/// Which of `dlls` Windows can't find (in the usual DLL search order, PATH included).
pub fn missing_dlls(dlls: &[&str]) -> Vec<String> {
    dlls.iter()
        .filter(|dll| match unsafe { LoadLibraryExW(&HSTRING::from(**dll), None, LOAD_LIBRARY_FLAGS(0)) } {
            Ok(module) => {
                unsafe {
                    let _ = FreeLibrary(module);
                }
                false
            }
            Err(_) => true,
        })
        .map(|dll| (*dll).to_owned())
        .collect()
}

/// Waits for `d` while answering this thread's window messages (see `clipboard::Owner`).
pub fn pump_for(d: Duration) {
    let end = Instant::now() + d;
    loop {
        unsafe {
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return;
            }
            MsgWaitForMultipleObjects(None, false, left.as_millis().max(1) as u32, QS_ALLINPUT);
        }
    }
}
