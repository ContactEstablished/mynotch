//! A global shortcut — modifiers plus one key — read from and shown as text like
//! "Ctrl+Alt+Space". Pure, so it's unit-tested; `app.rs` registers it with Windows.

use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hotkey {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
    /// Virtual-key code of the non-modifier key; always one with a name (see `key_name`).
    pub vk: u16,
}

/// Named keys besides letters, digits and F1–F24. Punctuation names are the US-layout labels
/// of those keys; on other layouts the label may differ, but the key code is what registers.
/// Tab, Esc, Enter and Backspace are left out on purpose: Esc and Backspace control the
/// recorder, and Tab/Enter combinations are too easy to hit by accident.
const KEYS: &[(u16, &str)] = &[
    (0x20, "Space"),
    (0x25, "Left"),
    (0x26, "Up"),
    (0x27, "Right"),
    (0x28, "Down"),
    (0x24, "Home"),
    (0x23, "End"),
    (0x21, "PageUp"),
    (0x22, "PageDown"),
    (0x2D, "Insert"),
    (0x2E, "Delete"),
    (0xBA, ";"),
    (0xBB, "="),
    (0xBC, ","),
    (0xBD, "-"),
    (0xBE, "."),
    (0xBF, "/"),
    (0xC0, "`"),
    (0xDB, "["),
    (0xDC, "\\"),
    (0xDD, "]"),
    (0xDE, "'"),
];

fn key_name(vk: u16) -> Option<String> {
    match vk {
        0x30..=0x39 | 0x41..=0x5A => Some(char::from(vk as u8).to_string()),
        0x70..=0x87 => Some(format!("F{}", vk - 0x6F)),
        _ => KEYS.iter().find(|&&(k, _)| k == vk).map(|&(_, name)| name.to_owned()),
    }
}

fn key_code(name: &str) -> Option<u16> {
    let upper = name.to_ascii_uppercase();
    if let [c] = upper.as_bytes()
        && c.is_ascii_alphanumeric()
    {
        return Some(u16::from(*c));
    }
    if let Some(n) = upper.strip_prefix('F').and_then(|n| n.parse::<u16>().ok())
        && (1..=24).contains(&n)
    {
        return Some(0x6F + n);
    }
    KEYS.iter().find(|(_, n)| n.eq_ignore_ascii_case(name)).map(|&(k, _)| k)
}

impl Hotkey {
    /// Ctrl+Alt+K.
    pub const DEFAULT: Hotkey = Hotkey { ctrl: true, alt: true, shift: false, win: false, vk: 0x4B };

    /// From a key pressed while recording; `None` unless valid.
    pub fn new(ctrl: bool, alt: bool, shift: bool, win: bool, vk: u16) -> Option<Hotkey> {
        Some(Hotkey { ctrl, alt, shift, win, vk }).filter(Hotkey::is_valid)
    }

    /// "Ctrl+Alt+Space": '+'-separated, modifiers in any order and case, spaces around parts
    /// allowed; "Control" and "Windows" work too. `None` unless valid.
    pub fn parse(s: &str) -> Option<Hotkey> {
        let parts: Vec<&str> = s.split('+').map(str::trim).collect();
        let (key, modifiers) = parts.split_last()?;
        let mut h = Hotkey { ctrl: false, alt: false, shift: false, win: false, vk: key_code(key)? };
        for m in modifiers {
            let flag = match m.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => &mut h.ctrl,
                "alt" => &mut h.alt,
                "shift" => &mut h.shift,
                "win" | "windows" => &mut h.win,
                _ => return None,
            };
            if std::mem::replace(flag, true) {
                return None;
            }
        }
        Some(h).filter(Hotkey::is_valid)
    }

    /// `RegisterHotKey`'s modifier bits (MOD_ALT 1, MOD_CONTROL 2, MOD_SHIFT 4, MOD_WIN 8).
    pub fn modifiers(&self) -> u32 {
        u32::from(self.alt) | u32::from(self.ctrl) << 1 | u32::from(self.shift) << 2 | u32::from(self.win) << 3
    }

    /// A named key with Win, or with at least two of Ctrl/Alt/Shift. A registered shortcut
    /// goes to MyNotch instead of the app in front, so single-modifier ones that apps rely on
    /// (Ctrl+C, Alt+F4) aren't allowed.
    fn is_valid(&self) -> bool {
        let held = [self.ctrl, self.alt, self.shift].into_iter().filter(|&m| m).count();
        (self.win || held >= 2) && key_name(self.vk).is_some()
    }
}

/// "Ctrl+Alt+Shift+Win+Key": modifiers in that order, then the key.
impl fmt::Display for Hotkey {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        for (on, name) in [(self.ctrl, "Ctrl"), (self.alt, "Alt"), (self.shift, "Shift"), (self.win, "Win")] {
            if on {
                write!(f, "{name}+")?;
            }
        }
        f.write_str(&key_name(self.vk).unwrap_or_else(|| format!("{:#04X}", self.vk)))
    }
}

/// Whether `vk` is a modifier on its own: Shift, Ctrl, Alt, the Windows keys, and their
/// left/right forms.
pub fn is_modifier(vk: u16) -> bool {
    matches!(vk, 0x10..=0x12 | 0x5B | 0x5C | 0xA0..=0xA5)
}

#[cfg(test)]
mod tests {
    use super::{Hotkey, KEYS, is_modifier, key_name};

    fn shown(s: &str) -> Option<String> {
        Hotkey::parse(s).map(|h| h.to_string())
    }

    #[test]
    fn default_is_ctrl_alt_k() {
        assert_eq!(Hotkey::DEFAULT.to_string(), "Ctrl+Alt+K");
        assert_eq!(Hotkey::parse("Ctrl+Alt+K"), Some(Hotkey::DEFAULT));
        assert_eq!(Hotkey::DEFAULT.modifiers(), 3);
    }

    #[test]
    fn parse_accepts_any_order_and_case() {
        assert_eq!(Hotkey::parse("alt + control + k"), Some(Hotkey::DEFAULT));
        assert_eq!(shown("alt + control + space").as_deref(), Some("Ctrl+Alt+Space"));
        assert_eq!(shown("win+shift+f12").as_deref(), Some("Shift+Win+F12"));
        assert_eq!(shown("Ctrl+Alt+pageup").as_deref(), Some("Ctrl+Alt+PageUp"));
        assert_eq!(shown("Windows+n").as_deref(), Some("Win+N"));
    }

    #[test]
    fn parse_rejects_invalid_combinations() {
        for bad in
            ["", "Space", "Shift+A", "Ctrl+C", "Alt+F4", "Ctrl+Alt", "Ctrl+Ctrl+A", "Ctrl+Alt+A+B", "Ctrl+Alt+Banana", "Ctrl+Alt+"]
        {
            assert_eq!(Hotkey::parse(bad), None, "{bad:?}");
        }
        for good in ["Ctrl+Alt+F4", "Ctrl+Shift+K", "Alt+Shift+Q", "Win+N"] {
            assert!(Hotkey::parse(good).is_some(), "{good:?}");
        }
    }

    #[test]
    fn every_key_name_round_trips() {
        let named = (0x30..=0x39).chain(0x41..=0x5A).chain(0x70..=0x87).chain(KEYS.iter().map(|&(k, _)| k));
        for vk in named {
            let name = key_name(vk).unwrap();
            let text = format!("Ctrl+Alt+{name}");
            let h = Hotkey::parse(&text).unwrap_or_else(|| panic!("{text}"));
            assert_eq!(h.vk, vk, "{text}");
            assert_eq!(h.to_string(), text);
        }
    }

    #[test]
    fn recorded_presses_need_win_or_two_modifiers() {
        assert_eq!(Hotkey::new(false, false, true, false, 0x41), None, "Shift+A");
        assert_eq!(Hotkey::new(true, false, false, false, 0x41), None, "Ctrl+A");
        assert!(Hotkey::new(true, false, true, false, 0x41).is_some(), "Ctrl+Shift+A");
        assert!(Hotkey::new(false, false, false, true, 0x41).is_some(), "Win+A");
        assert_eq!(Hotkey::new(true, true, false, false, 0x11), None, "a modifier is not a key");
        assert_eq!(Hotkey::new(true, true, false, false, 0x09), None, "Tab isn't allowed");
        for vk in [0x10, 0x11, 0x12, 0x5B, 0x5C, 0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5] {
            assert!(is_modifier(vk), "{vk:#x}");
        }
        assert!(!is_modifier(0x41));
    }
}
