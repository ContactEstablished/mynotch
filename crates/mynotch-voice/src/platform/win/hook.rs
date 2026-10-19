//! The low-level keyboard hook (`WH_KEYBOARD_LL`) on its own thread with its own message loop.
//! The callback runs `Trigger` and hands any signal to `notify`; it never blocks, because
//! Windows silently removes hooks that are slow to answer. It's re-installed after the PC
//! resumes from sleep.
//!
//! Known limitation: while an elevated (admin) window has the foreground, Windows doesn't call
//! a non-elevated hook at all, so holding Right Ctrl there does nothing (notes.md F8).

use std::cell::RefCell;
use std::ffi::c_void;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use tracing::{error, info, warn};
use windows::Win32::Foundation::{HANDLE, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Power::{
    DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS, RegisterSuspendResumeNotification, UnregisterSuspendResumeNotification,
};
use windows::Win32::System::Threading::{
    GetCurrentThread, GetCurrentThreadId, SetThreadPriority, THREAD_PRIORITY_TIME_CRITICAL,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DEVICE_NOTIFY_CALLBACK, DispatchMessageW, GetMessageW, HHOOK, KBDLLHOOKSTRUCT, LLKHF_EXTENDED,
    LLKHF_INJECTED, LLKHF_UP, MSG, PBT_APMRESUMEAUTOMATIC, PBT_APMSUSPEND, PM_NOREMOVE, PeekMessageW,
    PostThreadMessageW, SetWindowsHookExW, UnhookWindowsHookEx, WH_KEYBOARD_LL, WM_APP, WM_QUIT,
};

use super::input::{self, MARKER};
use crate::trigger::{KeyEvent, Replay, Signal, Trigger};

/// Posted to the hook thread by the power callback.
const WM_RESUMED: u32 = WM_APP + 1;
const WM_SUSPENDING: u32 = WM_APP + 2;

/// Hears the trigger's signals, on the hook thread, inside the hook: it must return at once.
pub type Notify = Box<dyn Fn(Signal, Instant) + Send>;

struct HookState {
    trigger: Trigger,
    notify: Notify,
}

thread_local! {
    static STATE: RefCell<Option<HookState>> = const { RefCell::new(None) };
}

/// The running hook. Dropping it removes the hook and ends its thread.
pub struct Hook {
    thread: u32,
    handle: thread::JoinHandle<()>,
}

impl Drop for Hook {
    fn drop(&mut self) {
        // A finished thread's id may belong to another thread by now.
        if !self.handle.is_finished() {
            unsafe {
                let _ = PostThreadMessageW(self.thread, WM_QUIT, WPARAM(0), LPARAM(0));
            }
        }
    }
}

/// Starts the hook thread and returns once the hook is installed.
pub fn install(accept_injected: bool, notify: Notify) -> Result<Hook, String> {
    let (ready_tx, ready_rx) = mpsc::channel();
    let handle = thread::Builder::new()
        .name("voice-hook".into())
        .spawn(move || run(Trigger::new(accept_injected), notify, ready_tx))
        .map_err(|e| e.to_string())?;
    let thread = ready_rx.recv().map_err(|_| "The hook thread stopped".to_owned())??;
    Ok(Hook { thread, handle })
}

fn run(trigger: Trigger, notify: Notify, ready: mpsc::Sender<Result<u32, String>>) {
    unsafe {
        // Give the thread its message queue before anyone posts to it.
        let mut msg = MSG::default();
        let _ = PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE);
        let thread = GetCurrentThreadId();
        // Windows removes a hook that is slow to answer; don't let a busy CPU starve it.
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_TIME_CRITICAL);
        STATE.with(|s| *s.borrow_mut() = Some(HookState { trigger, notify }));
        let mut hook = match set_hook() {
            Ok(hook) => hook,
            Err(e) => {
                let _ = ready.send(Err(e));
                return;
            }
        };
        info!("keyboard hook installed (thread {thread})");
        // The thread id rides in the callback's context pointer. The parameters must outlive
        // the registration.
        let params = Box::new(DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS {
            Callback: Some(on_power),
            Context: thread as usize as *mut c_void,
        });
        let power = RegisterSuspendResumeNotification(HANDLE(std::ptr::from_ref(&*params) as *mut c_void), DEVICE_NOTIFY_CALLBACK);
        if let Err(e) = &power {
            warn!("no sleep/resume notifications, so the hook won't be re-installed after sleep: {}", e.message());
        }
        let _ = ready.send(Ok(thread));

        while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
            match msg.message {
                WM_SUSPENDING => info!("the PC is going to sleep"),
                WM_RESUMED => {
                    let _ = UnhookWindowsHookEx(hook);
                    // Key-ups may have been missed while asleep.
                    let held = STATE.with(|s| s.borrow_mut().as_mut().and_then(|s| s.trigger.reset().map(|c| (s.notify)(c, Instant::now()))));
                    let note = if held.is_some() { "; the held session was cancelled" } else { "" };
                    match reinstall() {
                        Ok(h) => {
                            hook = h;
                            info!("resumed from sleep: keyboard hook re-installed{note}");
                        }
                        Err(e) => {
                            error!("resumed from sleep, but the keyboard hook couldn't be re-installed, so voice is off: {e}");
                            if let Ok(power) = power {
                                let _ = UnregisterSuspendResumeNotification(power);
                            }
                            return;
                        }
                    }
                }
                _ => {
                    DispatchMessageW(&msg);
                }
            }
        }
        let _ = UnhookWindowsHookEx(hook);
        if let Ok(power) = power {
            let _ = UnregisterSuspendResumeNotification(power);
        }
        drop(params);
        info!("keyboard hook removed");
    }
}

/// After a resume, Windows may not be ready for a moment: try for a few seconds.
unsafe fn reinstall() -> Result<HHOOK, String> {
    let mut last = Err(String::new());
    for _ in 0..5 {
        last = unsafe { set_hook() };
        if last.is_ok() {
            break;
        }
        thread::sleep(Duration::from_secs(1));
    }
    last
}

unsafe fn set_hook() -> Result<HHOOK, String> {
    unsafe {
        let module = GetModuleHandleW(None).map_err(|e| e.message())?;
        SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), Some(module.into()), 0).map_err(|e| e.message())
    }
}

unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let k = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        let e = KeyEvent {
            vk: k.vkCode,
            scan: k.scanCode,
            down: k.flags.0 & LLKHF_UP.0 == 0,
            extended: k.flags.0 & LLKHF_EXTENDED.0 != 0,
            injected: k.flags.0 & LLKHF_INJECTED.0 != 0,
            ours: k.dwExtraInfo == MARKER,
            time: k.time,
        };
        let decision = STATE.with(|s| {
            let mut s = s.try_borrow_mut().ok()?;
            let s = s.as_mut()?;
            let d = s.trigger.on_key(e);
            if let Some(signal) = d.signal {
                (s.notify)(signal, Instant::now());
            }
            Some(d)
        });
        if let Some(d) = decision {
            let replayed = match d.replay {
                Some(Replay::Chord) => input::replay_chord(&e),
                Some(Replay::Tap) => input::replay_tap(),
                None => Ok(()),
            };
            // A chord key whose replay couldn't be sent goes through as it is rather than be lost.
            if d.swallow && (replayed.is_ok() || d.replay != Some(Replay::Chord)) {
                return LRESULT(1);
            }
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

/// Sleep and resume, on a system thread: hand them to the hook thread.
unsafe extern "system" fn on_power(context: *const c_void, kind: u32, _setting: *const c_void) -> u32 {
    let message = match kind {
        PBT_APMRESUMEAUTOMATIC => WM_RESUMED,
        PBT_APMSUSPEND => WM_SUSPENDING,
        _ => return 0,
    };
    unsafe {
        let _ = PostThreadMessageW(context as usize as u32, message, WPARAM(0), LPARAM(0));
    }
    0
}
