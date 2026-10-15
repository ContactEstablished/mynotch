//! User preferences, persisted as `key=value` lines in `%APPDATA%\MyNotch\settings.ini`.
//! Unknown keys and malformed lines are ignored, so older and newer versions can share
//! the file. (Start with Windows lives in the registry instead; see `desktop::autostart`.)

use std::fs;
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub pinned: bool,
    pub hide_in_fullscreen: bool,
    /// Title of the last selected tab.
    pub tab: String,
    /// Length of a focus session (1..=180).
    pub focus_minutes: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self { pinned: false, hide_in_fullscreen: true, tab: String::new(), focus_minutes: 25 }
    }
}

impl Settings {
    pub fn load() -> Self {
        path().and_then(|p| fs::read_to_string(p).ok()).map_or_else(Self::default, |s| Self::parse(&s))
    }

    /// Best effort: a failed write just means the preference isn't remembered.
    pub fn save(&self) {
        let Some(path) = path() else { return };
        if let Some(dir) = path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        // Write a sibling file and rename it over, so a crash can't leave half a file.
        let tmp = path.with_extension("ini.tmp");
        if fs::write(&tmp, self.serialize()).is_ok() {
            let _ = fs::rename(&tmp, &path);
        }
    }

    fn parse(text: &str) -> Self {
        let mut s = Self::default();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else { continue };
            let value = value.trim();
            let flag = || match value {
                "true" => Some(true),
                "false" => Some(false),
                _ => None,
            };
            match key.trim() {
                "pinned" => s.pinned = flag().unwrap_or(s.pinned),
                "hide_in_fullscreen" => s.hide_in_fullscreen = flag().unwrap_or(s.hide_in_fullscreen),
                "tab" => s.tab = value.to_string(),
                "focus_minutes" => {
                    if let Ok(m) = value.parse::<u32>() {
                        s.focus_minutes = m.clamp(1, 180);
                    }
                }
                _ => {}
            }
        }
        s
    }

    fn serialize(&self) -> String {
        format!(
            "pinned={}\nhide_in_fullscreen={}\ntab={}\nfocus_minutes={}\n",
            self.pinned, self.hide_in_fullscreen, self.tab, self.focus_minutes
        )
    }
}

fn path() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|dir| PathBuf::from(dir).join("MyNotch").join("settings.ini"))
}

#[cfg(test)]
mod tests {
    use super::Settings;

    #[test]
    fn round_trips() {
        let s = Settings { pinned: true, hide_in_fullscreen: false, tab: "System".into(), focus_minutes: 50 };
        assert_eq!(Settings::parse(&s.serialize()), s);
    }

    #[test]
    fn tolerates_junk_and_unknown_keys() {
        let s = Settings::parse("pinned = true\nfuture_option=7\nno equals sign\nhide_in_fullscreen=maybe\nfocus_minutes=-3\n");
        assert_eq!(s, Settings { pinned: true, ..Settings::default() });
    }

    #[test]
    fn focus_minutes_is_clamped() {
        assert_eq!(Settings::parse("focus_minutes=999").focus_minutes, 180);
        assert_eq!(Settings::parse("focus_minutes=0").focus_minutes, 1);
    }

    #[test]
    fn empty_file_gives_defaults() {
        assert_eq!(Settings::parse(""), Settings::default());
    }
}
