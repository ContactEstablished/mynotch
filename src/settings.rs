//! User preferences, persisted as `key=value` lines in `%APPDATA%\MyNotch\settings.ini`.
//! Unknown keys and malformed lines are ignored, so older and newer versions can share
//! the file. (Start with Windows lives in the registry instead; see `desktop::autostart`.)

use std::fs;
use std::path::PathBuf;

/// The default body colour, obsidian, as `0xRRGGBB`.
pub const DEFAULT_COLOR: u32 = 0x0B0912;
/// The most see-through the body may be, in percent.
pub const OPACITY_MIN: u32 = 10;

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub pinned: bool,
    pub hide_in_fullscreen: bool,
    /// Title of the last selected tab.
    pub tab: String,
    /// Length of a focus session (1..=180).
    pub focus_minutes: u32,
    /// Apps pinned in the Apps tab, by shell id, in order. `None` until first chosen, so the
    /// tab can suggest some on first run.
    pub apps: Option<Vec<String>>,
    /// Body colour, `0xRRGGBB`.
    pub color: u32,
    /// Body opacity in percent (`OPACITY_MIN..=100`); content stays fully opaque.
    pub opacity: u32,
    /// Where the notch sits along the top of the screen: 0 = flush left, 1 = flush right.
    pub position: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            pinned: false,
            hide_in_fullscreen: true,
            tab: String::new(),
            focus_minutes: 25,
            apps: None,
            color: DEFAULT_COLOR,
            opacity: 100,
            position: 0.5,
        }
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
                // '|' can't appear in a path or an AppUserModelID.
                "apps" => s.apps = Some(value.split('|').filter(|id| !id.is_empty()).map(String::from).collect()),
                "color" => {
                    if let Ok(c) = u32::from_str_radix(value, 16)
                        && c <= 0xFFFFFF
                    {
                        s.color = c;
                    }
                }
                "opacity" => {
                    if let Ok(o) = value.parse::<u32>() {
                        s.opacity = o.clamp(OPACITY_MIN, 100);
                    }
                }
                "position" => {
                    if let Ok(p) = value.parse::<f32>()
                        && p.is_finite()
                    {
                        s.position = p.clamp(0.0, 1.0);
                    }
                }
                _ => {}
            }
        }
        s
    }

    fn serialize(&self) -> String {
        let mut text = format!(
            "pinned={}\nhide_in_fullscreen={}\ntab={}\nfocus_minutes={}\ncolor={:06X}\nopacity={}\nposition={}\n",
            self.pinned,
            self.hide_in_fullscreen,
            self.tab,
            self.focus_minutes,
            self.color,
            self.opacity,
            self.position
        );
        if let Some(apps) = &self.apps {
            text += &format!("apps={}\n", apps.join("|"));
        }
        text
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
        let s = Settings {
            pinned: true,
            hide_in_fullscreen: false,
            tab: "System".into(),
            focus_minutes: 50,
            apps: None,
            color: 0x14284A,
            opacity: 65,
            position: 0.3125,
        };
        assert_eq!(Settings::parse(&s.serialize()), s);
        let apps = vec!["Microsoft.WindowsTerminal_8wekyb3d8bbwe!App".to_string(), "Brave".to_string()];
        let s = Settings { apps: Some(apps), ..s };
        assert_eq!(Settings::parse(&s.serialize()), s);
        let s = Settings { apps: Some(Vec::new()), ..s };
        assert_eq!(Settings::parse(&s.serialize()), s, "all unpinned stays distinct from never set");
    }

    #[test]
    fn tolerates_junk_and_unknown_keys() {
        let s = Settings::parse(
            "pinned = true\nfuture_option=7\nno equals sign\nhide_in_fullscreen=maybe\nfocus_minutes=-3\n\
             color=purple\nopacity=lots\nposition=NaN\n",
        );
        assert_eq!(s, Settings { pinned: true, ..Settings::default() });
        assert_eq!(Settings::parse("color=1000000").color, Settings::default().color, "more than 24 bits");
    }

    #[test]
    fn numbers_are_clamped() {
        assert_eq!(Settings::parse("focus_minutes=999").focus_minutes, 180);
        assert_eq!(Settings::parse("focus_minutes=0").focus_minutes, 1);
        assert_eq!(Settings::parse("opacity=0").opacity, super::OPACITY_MIN);
        assert_eq!(Settings::parse("opacity=250").opacity, 100);
        assert_eq!(Settings::parse("position=-2").position, 0.0);
        assert_eq!(Settings::parse("position=7.5").position, 1.0);
    }

    #[test]
    fn empty_file_gives_defaults() {
        assert_eq!(Settings::parse(""), Settings::default());
    }
}
