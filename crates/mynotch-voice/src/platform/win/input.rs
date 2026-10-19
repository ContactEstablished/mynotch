//! Synthesized keyboard input. Everything MyNotch injects carries `MARKER` in `dwExtraInfo`, so
//! its own hook never mistakes it for the user's keys.

use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY,
    KEYEVENTF_KEYUP, MAPVK_VK_TO_VSC, MapVirtualKeyW, SendInput, VIRTUAL_KEY, VK_CONTROL, VK_LWIN, VK_MENU,
    VK_RCONTROL, VK_RWIN, VK_SHIFT,
};

use crate::trigger::KeyEvent;

/// "MNVC": tags every event MyNotch injects.
pub const MARKER: usize = 0x4D4E_5643;
/// Right Ctrl's scan code (it's an extended key).
const RCTRL_SCAN: u16 = 0x1D;
const V: VIRTUAL_KEY = VIRTUAL_KEY(0x56);

fn key(vk: VIRTUAL_KEY, scan: u16, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, wScan: scan, dwFlags: flags, time: 0, dwExtraInfo: MARKER } },
    }
}

fn scan_of(vk: VIRTUAL_KEY) -> u16 {
    unsafe { MapVirtualKeyW(u32::from(vk.0), MAPVK_VK_TO_VSC) as u16 }
}

fn send(inputs: &[INPUT]) -> Result<(), String> {
    let sent = unsafe { SendInput(inputs, size_of::<INPUT>() as i32) } as usize;
    if sent == inputs.len() {
        return Ok(());
    }
    // Blocked by UIPI (the target runs elevated) or another app's input lock. Don't leave a
    // key down that went in without its up.
    let mut down: Vec<KEYBDINPUT> = Vec::new();
    for input in &inputs[..sent] {
        let ki = unsafe { input.Anonymous.ki };
        if ki.dwFlags.contains(KEYEVENTF_KEYUP) {
            down.retain(|d| d.wVk != ki.wVk);
        } else {
            down.push(ki);
        }
    }
    let ups: Vec<INPUT> = down.iter().rev().map(|d| key(d.wVk, d.wScan, d.dwFlags | KEYEVENTF_KEYUP)).collect();
    if !ups.is_empty() {
        unsafe { SendInput(&ups, size_of::<INPUT>() as i32) };
    }
    Err(format!("Windows took {sent} of {} keys", inputs.len()))
}

/// Ctrl+V.
pub fn send_paste() -> Result<(), String> {
    let (ctrl, v) = (scan_of(VK_CONTROL), scan_of(V));
    send(&[
        key(VK_CONTROL, ctrl, KEYBD_EVENT_FLAGS(0)),
        key(V, v, KEYBD_EVENT_FLAGS(0)),
        key(V, v, KEYEVENTF_KEYUP),
        key(VK_CONTROL, ctrl, KEYEVENTF_KEYUP),
    ])
}

/// A chord's opening: Right Ctrl down, then the key that made it a chord, in that order.
pub(super) fn replay_chord(e: &KeyEvent) -> Result<(), String> {
    let extended = if e.extended { KEYEVENTF_EXTENDEDKEY } else { KEYBD_EVENT_FLAGS(0) };
    send(&[
        key(VK_RCONTROL, RCTRL_SCAN, KEYEVENTF_EXTENDEDKEY),
        key(VIRTUAL_KEY(e.vk as u16), e.scan as u16, extended),
    ])
}

/// A Right Ctrl tap, given back to the app after the hook swallowed it.
pub(super) fn replay_tap() -> Result<(), String> {
    send(&[
        key(VK_RCONTROL, RCTRL_SCAN, KEYEVENTF_EXTENDEDKEY),
        key(VK_RCONTROL, RCTRL_SCAN, KEYEVENTF_EXTENDEDKEY | KEYEVENTF_KEYUP),
    ])
}

/// Whether Shift, Ctrl, Alt or Win is down (as Windows sees it: the swallowed Right Ctrl isn't).
pub fn modifiers_held() -> bool {
    [VK_SHIFT, VK_CONTROL, VK_MENU, VK_LWIN, VK_RWIN].iter().any(|vk| unsafe { GetAsyncKeyState(i32::from(vk.0)) } < 0)
}
