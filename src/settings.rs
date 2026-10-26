//! User preferences, persisted as `key=value` lines in `%APPDATA%\MyNotch\settings.ini`.
//! Unknown keys and malformed lines are ignored, so older and newer versions can share
//! the file. (Start with Windows lives in the registry instead; see `desktop::autostart`.)

use std::fs;
use std::path::PathBuf;

use crate::awake::Length as AwakeLength;
use crate::hotkey::Hotkey;
use crate::weather::Place;
use crate::widgets::TimerStyle;

/// The default body colour, the Halo glass tint (`theme::color::GLASS`), as `0xRRGGBB`.
pub const DEFAULT_COLOR: u32 = crate::theme::color::GLASS.hex();
/// The default body opacity, in percent: see-through enough for the wallpaper to show.
pub const DEFAULT_OPACITY: u32 = 60;
/// The default colour before the Halo look.
pub const OBSIDIAN: u32 = 0x0B0912;
/// Written to every saved file, so a file without it predates the Halo look.
const STYLE: &str = "halo";
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
    /// How the timer looks.
    pub timer_style: TimerStyle,
    /// Apps pinned in the Apps tab, by shell id, in order. `None` until first chosen, so the
    /// tab can suggest some on first run.
    pub apps: Option<Vec<String>>,
    /// Body colour, `0xRRGGBB`.
    pub color: u32,
    /// Body opacity in percent (`OPACITY_MIN..=100`); content stays fully opaque.
    pub opacity: u32,
    /// Where the notch sits along the top of the screen: 0 = flush left, 1 = flush right.
    pub position: f32,
    /// The global shortcut that opens app search; `None` = no shortcut.
    pub hotkey: Option<Hotkey>,
    /// Where the weather is for: what the user typed, and the place it was found to be
    /// (`None` until looked up, or if nothing matched).
    pub place: Option<(String, Option<Place>)>,
    /// How long keep awake stays on. (Whether it's on isn't saved: it always starts off.)
    pub awake_length: AwakeLength,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            pinned: false,
            hide_in_fullscreen: true,
            tab: String::new(),
            focus_minutes: 25,
            timer_style: TimerStyle::default(),
            apps: None,
            color: DEFAULT_COLOR,
            opacity: DEFAULT_OPACITY,
            position: 0.5,
            hotkey: Some(Hotkey::DEFAULT),
            place: None,
            awake_length: AwakeLength::default(),
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
        let mut styled = false;
        let (mut query, mut name, mut lat, mut lon) = (None, None, None, None);
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
                // A style this version doesn't know falls back to the default.
                "timer_style" => s.timer_style = TimerStyle::from_key(value).unwrap_or_default(),
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
                // Empty means no shortcut; one that can't be read falls back to the default
                // rather than quietly turning the shortcut off.
                "hotkey" if value.is_empty() => s.hotkey = None,
                "hotkey" => s.hotkey = Some(Hotkey::parse(value).unwrap_or(Hotkey::DEFAULT)),
                "awake_length" => s.awake_length = AwakeLength::parse(value).unwrap_or_default(),
                "style" => styled = value == STYLE,
                "place" if !value.is_empty() => query = Some(value.to_string()),
                "place_name" => name = Some(value.to_string()),
                "place_lat" => lat = value.parse::<f64>().ok().filter(|v| (-90.0..=90.0).contains(v)),
                "place_lon" => lon = value.parse::<f64>().ok().filter(|v| (-180.0..=180.0).contains(v)),
                _ => {}
            }
        }
        s.place = query.map(|q| {
            let found = match (name, lat, lon) {
                (Some(name), Some(lat), Some(lon)) => Some(Place { name, lat, lon }),
                _ => None,
            };
            (q, found)
        });
        // Saved before the Halo look, with the colour never changed from the old default:
        // move to the new default glass, once.
        if !styled && s.color == OBSIDIAN {
            (s.color, s.opacity) = (DEFAULT_COLOR, DEFAULT_OPACITY);
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
        // Always written, so "none" stays distinct from "never set" (the default).
        text += &format!("hotkey={}\n", self.hotkey.map(|h| h.to_string()).unwrap_or_default());
        if let Some((query, found)) = &self.place {
            text += &format!("place={query}\n");
            if let Some(p) = found {
                text += &format!("place_name={}\nplace_lat={}\nplace_lon={}\n", p.name, p.lat, p.lon);
            }
        }
        text += &format!("timer_style={}\n", self.timer_style.key());
        text += &format!("awake_length={}\n", self.awake_length.key());
        text += &format!("style={STYLE}\n");
        text
    }
}

fn path() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|dir| PathBuf::from(dir).join("MyNotch").join("settings.ini"))
}

#[cfg(test)]
mod tests {
    use super::Settings;
    use crate::awake::Length as AwakeLength;
    use crate::weather::Place;
    use crate::hotkey::Hotkey;
    use crate::widgets::TimerStyle;

    #[test]
    fn round_trips() {
        let s = Settings {
            pinned: true,
            hide_in_fullscreen: false,
            tab: "System".into(),
            focus_minutes: 50,
            timer_style: TimerStyle::Retro,
            apps: None,
            color: 0x14284A,
            opacity: 65,
            position: 0.3125,
            hotkey: Some(Hotkey { ctrl: true, alt: false, shift: true, win: false, vk: 0x4B }),
            place: Some(("02139".into(), Some(Place { name: "Cambridge, Massachusetts".into(), lat: 42.3751, lon: -71.10561 }))),
            awake_length: AwakeLength::Minutes(135),
        };
        assert_eq!(Settings::parse(&s.serialize()), s);
        let apps = vec!["Microsoft.WindowsTerminal_8wekyb3d8bbwe!App".to_string(), "Brave".to_string()];
        let s = Settings { apps: Some(apps), ..s };
        assert_eq!(Settings::parse(&s.serialize()), s);
        let s = Settings { apps: Some(Vec::new()), ..s };
        assert_eq!(Settings::parse(&s.serialize()), s, "all unpinned stays distinct from never set");
        let s = Settings { hotkey: None, ..s };
        assert_eq!(Settings::parse(&s.serialize()), s, "no shortcut stays distinct from never set");
        let s = Settings { place: Some(("Atlantis".into(), None)), ..s };
        assert_eq!(Settings::parse(&s.serialize()), s, "a place not found yet keeps its query");
        let s = Settings { place: None, ..s };
        assert_eq!(Settings::parse(&s.serialize()), s);
        let s = Settings { awake_length: AwakeLength::Forever, ..s };
        assert_eq!(Settings::parse(&s.serialize()), s, "no limit");
    }

    #[test]
    fn hotkey_defaults_clears_and_falls_back() {
        assert_eq!(Settings::parse("").hotkey, Some(Hotkey::DEFAULT));
        assert_eq!(Settings::parse("hotkey=").hotkey, None);
        assert_eq!(Settings::parse("hotkey=Ctrl+Shift+K").hotkey, Hotkey::parse("Ctrl+Shift+K"));
        assert_eq!(Settings::parse("hotkey=Shift+K").hotkey, Some(Hotkey::DEFAULT));
        assert_eq!(Settings::parse("hotkey=nonsense").hotkey, Some(Hotkey::DEFAULT));
    }

    #[test]
    fn tolerates_junk_and_unknown_keys() {
        let s = Settings::parse(
            "pinned = true\nfuture_option=7\nno equals sign\nhide_in_fullscreen=maybe\nfocus_minutes=-3\n\
             color=purple\nopacity=lots\nposition=NaN\n",
        );
        assert_eq!(s, Settings { pinned: true, ..Settings::default() });
        assert_eq!(Settings::parse("color=1000000").color, Settings::default().color, "more than 24 bits");
        assert_eq!(Settings::parse("timer_style=plaid").timer_style, TimerStyle::default(), "a style from the future");
        assert_eq!(Settings::parse("awake_length=soon").awake_length, AwakeLength::default());
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
    fn an_untouched_obsidian_notch_moves_to_halo_once() {
        let old = Settings::parse("color=0B0912\nopacity=35\n");
        assert_eq!((old.color, old.opacity), (super::DEFAULT_COLOR, super::DEFAULT_OPACITY));
        let picked = Settings { color: super::OBSIDIAN, opacity: 35, ..Settings::default() };
        assert_eq!(Settings::parse(&picked.serialize()), picked, "obsidian picked since then stays");
        let custom = Settings::parse("color=14285A\nopacity=35\n");
        assert_eq!((custom.color, custom.opacity), (0x14285A, 35), "a colour the user picked stays");
    }

    #[test]
    fn empty_file_gives_defaults() {
        assert_eq!(Settings::parse(""), Settings::default());
    }
}
