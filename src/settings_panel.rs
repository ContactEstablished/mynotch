//! The Settings tab: the notch's colour, opacity, position and global shortcut.

use std::cell::Cell;

use windows::{
    Win32::{
        Foundation::{COLORREF, HWND, LPARAM, RECT, WPARAM},
        UI::{
            Controls::Dialogs::{
                CC_ANYCOLOR, CC_ENABLEHOOK, CC_FULLOPEN, CC_RGBINIT, CHOOSECOLORW, COLOR_BLUE, COLOR_GREEN, COLOR_RED,
                ChooseColorW,
            },
            WindowsAndMessaging::{
                EN_CHANGE, GetDlgItemInt, GetWindowRect, SWP_NOSIZE, SWP_NOZORDER, SetForegroundWindow, SetWindowPos,
                WM_COMMAND, WM_INITDIALOG,
            },
        },
    },
    core::BOOL,
};

use crate::gfx::{Align, Canvas, Font, Ink, Look, Rect, Rgb, glyph};
use crate::theme::{color, space};
use crate::settings::{OPACITY_MIN, Settings};
use crate::text::{self, TextField};
use crate::weather::Status;
use crate::widgets::{slider, slider_at, slider_hit};

/// Colour presets, deep enough for white text to stay readable. The first is the default.
pub const PRESETS: [(&str, u32); 10] = [
    ("Halo", crate::settings::DEFAULT_COLOR),
    ("Obsidian", crate::settings::OBSIDIAN),
    ("Graphite", 0x2A2A30),
    ("Slate", 0x26344A),
    ("Navy", 0x14285A),
    ("Ocean", 0x0A4552),
    ("Forest", 0x123F26),
    ("Plum", 0x3E1A52),
    ("Wine", 0x5A1426),
    ("Ember", 0x6A2A0E),
];
/// Opacity moves in steps of this many percent.
const STEP: u32 = 5;

/// Five rows fill the 146 px content area: 0.5 + 5 × 29.
const ROW_H: f32 = 29.0;
const ROW_Y: f32 = 0.5;
const ROWS: f32 = 5.0;
// 146 is the content area's height (`CONTENT` in app.rs).
const _: () = assert!(ROW_Y + ROW_H * ROWS <= 146.0, "the Settings rows must fit the panel");
const LABEL_W: f32 = 96.0;
const SWATCH: f32 = 24.0;
const SWATCH_PITCH: f32 = 34.0;
/// Presets, then the custom-colour swatch.
const SWATCHES: usize = PRESETS.len() + 1;
/// Room kept at the slider's ends for its knob.
const KNOB_R: f32 = 8.0;
const CHIP: (f32, f32) = (76.0, 24.0);
/// Wide enough for "Ctrl+Alt+Shift+PageDown".
const SHORTCUT_W: f32 = 168.0;
/// The weather place field.
const PLACE_W: f32 = 200.0;
/// Longest place, in chars.
pub const PLACE_MAX: usize = 60;

/// What a click on the panel landed on.
pub enum Hit {
    Preset(u32),
    /// The custom swatch: open Windows' colour picker.
    Custom,
    /// The opacity slider; see `opacity_at`.
    Opacity,
    Center,
    /// The Shortcut chip: start recording a new shortcut.
    Shortcut,
    /// The weather place field: start typing a city or ZIP.
    Place,
}

/// What the Weather row shows besides the saved place.
pub struct PlaceRow<'a> {
    /// The field being typed in, and whether its caret is showing.
    pub editing: Option<(&'a TextField, bool)>,
    pub status: Status,
}

/// What the Shortcut row shows besides the saved shortcut.
#[derive(Clone, Copy, Default)]
pub struct ShortcutRow {
    /// Waiting for the user to press a combination.
    pub recording: bool,
    /// Windows refused to register the saved shortcut.
    pub taken: bool,
}

pub fn look(settings: &Settings) -> Look {
    Look { color: Rgb::from_hex(settings.color), opacity: settings.opacity as f32 / 100.0 }
}

fn row_centre(i: usize) -> f32 {
    ROW_Y + ROW_H * (i as f32 + 0.5)
}

fn swatch(i: usize) -> Rect {
    let x = LABEL_W + i as f32 * SWATCH_PITCH;
    Rect::new(x, row_centre(0) - SWATCH / 2.0, SWATCH, SWATCH)
}

/// The slider's track; the knob's centre travels its length.
fn track() -> Rect {
    let x = LABEL_W + KNOB_R;
    let right = swatch(SWATCHES - 1).right() - KNOB_R;
    Rect::new(x, row_centre(1) - 2.0, right - x, 4.0)
}

fn center_chip() -> Rect {
    Rect::new(LABEL_W, row_centre(2) - CHIP.1 / 2.0, CHIP.0, CHIP.1)
}

fn shortcut_chip() -> Rect {
    Rect::new(LABEL_W, row_centre(3) - CHIP.1 / 2.0, SHORTCUT_W, CHIP.1)
}

/// The weather place field.
fn place_field() -> Rect {
    Rect::new(LABEL_W, row_centre(4) - CHIP.1 / 2.0, PLACE_W, CHIP.1)
}

/// Where the place field's text goes, inside its padding.
pub fn place_text_rect() -> Rect {
    let f = place_field();
    Rect::new(f.x + 10.0, f.y, f.w - 20.0, f.h)
}

/// The opacity the slider gives at content x `x`.
pub fn opacity_at(x: f32) -> u32 {
    let o = OPACITY_MIN as f32 + slider_at(track(), x) * (100 - OPACITY_MIN) as f32;
    ((o / STEP as f32).round() as u32 * STEP).clamp(OPACITY_MIN, 100)
}

/// The wheel over the slider: a step per notch, positive = more opaque.
pub fn wheel_opacity(x: f32, y: f32, opacity: u32, notches: f32) -> Option<u32> {
    let steps = notches.round() as i32;
    (slider_hit(track()).contains(x, y) && steps != 0)
        .then(|| (opacity as i32 + steps * STEP as i32).clamp(OPACITY_MIN as i32, 100) as u32)
}

pub fn hit(x: f32, y: f32) -> Option<Hit> {
    if let Some(i) = (0..SWATCHES).find(|&i| swatch(i).contains(x, y)) {
        return Some(PRESETS.get(i).map_or(Hit::Custom, |&(_, c)| Hit::Preset(c)));
    }
    if slider_hit(track()).contains(x, y) {
        return Some(Hit::Opacity);
    }
    if shortcut_chip().contains(x, y) {
        return Some(Hit::Shortcut);
    }
    if place_field().contains(x, y) {
        return Some(Hit::Place);
    }
    center_chip().contains(x, y).then_some(Hit::Center)
}

pub fn draw(cv: &Canvas, w: f32, settings: &Settings, shortcut: ShortcutRow, place: PlaceRow) {
    let label = |i: usize, text: &str| {
        cv.caps(text, Font::Caption, Ink::Dim, Rect::new(0.0, row_centre(i) - 12.0, LABEL_W, 24.0), Align::Left);
    };
    let after = |r: Rect| Rect::new(r.right() + 14.0, r.y + r.h / 2.0 - 12.0, w - r.right() - 14.0, 24.0);
    // A chip: glass at rest, brighter under the cursor, lit while it's taking input.
    let chip_bg = |r: Rect, enabled: bool, lit: bool| {
        cv.fill(r, space::CONTROL_RADIUS, Ink::Track);
        cv.fill_a(r, space::CONTROL_RADIUS, Ink::Hover, cv.hover_if(r, enabled && !lit));
        if lit {
            cv.fill(r, space::CONTROL_RADIUS, Ink::Lit);
            cv.outline(r, space::CONTROL_RADIUS, Ink::LitEdge, 1.0);
        }
    };

    // Colour: presets, then a custom swatch showing the custom colour once one is chosen.
    label(0, "Color");
    let preset = PRESETS.iter().position(|&(_, c)| c == settings.color);
    for i in 0..SWATCHES {
        let r = swatch(i);
        let (cx, cy, radius) = (r.x + r.w / 2.0, r.y + r.h / 2.0, r.w / 2.0);
        let selected = preset.map_or(i == SWATCHES - 1, |p| p == i);
        match PRESETS.get(i) {
            Some(&(_, c)) => cv.circle(cx, cy, radius, Ink::Rgb(Rgb::from_hex(c)), None),
            None if preset.is_none() => cv.circle(cx, cy, radius, Ink::Rgb(Rgb::from_hex(settings.color)), None),
            None => cv.circle(cx, cy, radius, Ink::Track, None),
        }
        if i == SWATCHES - 1 {
            cv.text(glyph::PALETTE, Font::Icon, Ink::Text, r, Align::Center);
        }
        // Dark swatches would vanish into the body without an edge.
        cv.circle(cx, cy, radius - 0.5, Ink::Edge, Some(1.0));
        if selected {
            cv.circle(cx, cy, radius + 3.5, Ink::Accent, Some(1.5));
            cv.halo(cx, cy, radius + 3.5, Ink::Accent, 0.6);
        } else {
            cv.circle_a(cx, cy, radius + 3.5, Ink::Dim, Some(1.0), cv.hover(r));
        }
    }
    let name = preset.map_or("Custom", |p| PRESETS[p].0);
    cv.text(name, Font::Body, Ink::Text, after(swatch(SWATCHES - 1)), Align::Left);

    // Opacity: a slider from OPACITY_MIN to 100%.
    label(1, "Opacity");
    let t = track();
    let k = settings.opacity.saturating_sub(OPACITY_MIN) as f32 / (100 - OPACITY_MIN) as f32;
    slider(cv, t, k, cv.hovered(slider_hit(t)), true);
    let value = after(Rect { w: t.w + KNOB_R, ..t });
    cv.text(&format!("{}%", settings.opacity), Font::Body, Ink::Text, value, Align::Left);

    // Position: back to the middle; sliding happens by dragging the tab row.
    label(2, "Position");
    let chip = center_chip();
    let centred = settings.position == 0.5;
    chip_bg(chip, !centred, false);
    cv.text("Center", Font::Label, if centred { Ink::Dim } else { Ink::Text }, chip, Align::Center);
    let hint = "Or drag the empty space beside the tabs to slide the notch";
    cv.text(hint, Font::Small, Ink::Dim, after(chip), Align::Left);

    // Shortcut: click the chip, then press the new combination. While it listens, it's lit.
    label(3, "Shortcut");
    let chip = shortcut_chip();
    chip_bg(chip, true, shortcut.recording);
    let saved = settings.hotkey.map(|h| h.to_string());
    let (text, ink, note, note_ink) = match (&saved, shortcut) {
        (_, ShortcutRow { recording: true, .. }) => {
            ("Press a shortcut…", Ink::Accent, "Esc cancels · Backspace clears", Ink::Dim)
        }
        (Some(s), ShortcutRow { taken: true, .. }) => {
            (s.as_str(), Ink::Text, "Taken by another app", Ink::Rgb(color::WARN))
        }
        (Some(s), _) => (s.as_str(), Ink::Text, "Opens app search from anywhere", Ink::Dim),
        (None, _) => ("None", Ink::Dim, "Click to set a shortcut", Ink::Dim),
    };
    cv.text(text, Font::Label, ink, chip, Align::Center);
    cv.text(note, Font::Small, note_ink, after(chip), Align::Left);

    // Weather: click the field and type a city or ZIP; Enter looks it up.
    label(4, "Weather");
    let field = place_field();
    chip_bg(field, true, place.editing.is_some());
    let placeholder = "City or ZIP code";
    let query = settings.place.as_ref().map(|(q, _)| q.as_str());
    match place.editing {
        Some((typed, caret_on)) => text::draw(cv, typed, place_text_rect(), Font::Body, placeholder, caret_on),
        None => {
            let (shown, ink) = query.map_or((placeholder, Ink::Dim), |q| (q, Ink::Text));
            cv.text(shown, Font::Body, ink, place_text_rect(), Align::Left);
        }
    }
    let found = settings.place.as_ref().and_then(|(_, found)| found.as_ref());
    let (note, note_ink) = match (place.editing, found, place.status) {
        (Some(_), _, _) => ("Enter saves · Esc cancels · empty turns it off".to_owned(), Ink::Dim),
        (None, Some(p), Status::Offline) => (format!("{} · can't reach the weather service", p.name), Ink::Dim),
        (None, Some(p), _) => (p.name.clone(), Ink::Text),
        (None, None, _) if query.is_none() => ("Shows the temperature and sky on the Time tab".to_owned(), Ink::Dim),
        (None, None, Status::NotFound) => ("No place found by that name".to_owned(), Ink::Rgb(color::WARN)),
        (None, None, Status::Offline) => ("Can't reach the weather service".to_owned(), Ink::Dim),
        (None, None, _) => ("Looking it up…".to_owned(), Ink::Dim),
    };
    cv.text(&note, Font::Small, note_ink, after(field), Align::Left);
}

fn colorref(c: u32) -> COLORREF {
    let Rgb(r, g, b) = Rgb::from_hex(c);
    COLORREF(u32::from(r) | u32::from(g) << 8 | u32::from(b) << 16)
}

/// Where Windows' colour picker opens, and who sees the colours tried in it.
#[derive(Clone, Copy)]
pub struct Picker {
    /// Screen x the dialog centres on, and its top edge.
    pub centre: i32,
    pub top: i32,
    /// Screen x range the dialog stays within.
    pub left: i32,
    pub right: i32,
    /// Called with each colour the user tries (`0xRRGGBB`), for a live preview.
    pub preview: fn(u32),
}

thread_local! {
    /// The open picker, and whether its dialog has finished setting itself up.
    static OPEN: Cell<Option<(Picker, bool)>> = const { Cell::new(None) };
}

/// Places the dialog, then previews every colour the user tries: picking from the spectrum
/// or the swatches, or typing, all end up rewriting the Red/Green/Blue boxes.
unsafe extern "system" fn picker_hook(dlg: HWND, msg: u32, wparam: WPARAM, _lparam: LPARAM) -> usize {
    let Some((picker, ready)) = OPEN.get() else { return 0 };
    match msg {
        WM_INITDIALOG => {
            let mut r = RECT::default();
            unsafe {
                if GetWindowRect(dlg, &mut r).is_ok() {
                    let w = r.right - r.left;
                    let x = (picker.centre - w / 2).clamp(picker.left, (picker.right - w).max(picker.left));
                    let _ = SetWindowPos(dlg, None, x, picker.top, 0, 0, SWP_NOSIZE | SWP_NOZORDER);
                }
                let _ = SetForegroundWindow(dlg);
            }
            OPEN.set(Some((picker, true)));
            // Let the dialog set its default focus.
            1
        }
        // Edits made while the dialog fills itself in show only the starting colour, partly.
        WM_COMMAND if ready && (wparam.0 >> 16) as u32 & 0xFFFF == EN_CHANGE => {
            let boxes = [COLOR_RED, COLOR_GREEN, COLOR_BLUE];
            if boxes.contains(&((wparam.0 & 0xFFFF) as u32)) {
                let mut all = true;
                let [r, g, b] = boxes.map(|id| {
                    let mut ok = BOOL(0);
                    let v = unsafe { GetDlgItemInt(dlg, id as i32, Some(&mut ok), false) };
                    all &= ok.as_bool();
                    v.min(255) as u8
                });
                if all {
                    (picker.preview)(Rgb(r, g, b).hex());
                }
            }
            0
        }
        _ => 0,
    }
}

/// Windows' colour picker, starting at `initial`. Runs a modal loop; `None` if cancelled.
pub fn pick_color(owner: HWND, initial: u32, picker: Picker) -> Option<u32> {
    thread_local! {
        /// The picker's sixteen "custom colours", kept for the session; they start as the presets.
        static CUSTOM: Cell<[COLORREF; 16]> = Cell::new(std::array::from_fn(|i| {
            PRESETS.get(i).map_or(COLORREF(0xFFFFFF), |&(_, c)| colorref(c))
        }));
    }
    let mut custom = CUSTOM.get();
    let mut cc = CHOOSECOLORW {
        lStructSize: size_of::<CHOOSECOLORW>() as u32,
        hwndOwner: owner,
        rgbResult: colorref(initial),
        lpCustColors: custom.as_mut_ptr(),
        Flags: CC_ANYCOLOR | CC_FULLOPEN | CC_RGBINIT | CC_ENABLEHOOK,
        lpfnHook: Some(picker_hook),
        ..Default::default()
    };
    OPEN.set(Some((picker, false)));
    let picked = unsafe { ChooseColorW(&mut cc) }.as_bool();
    OPEN.set(None);
    CUSTOM.set(custom);
    picked.then(|| {
        let v = cc.rgbResult.0;
        Rgb(v as u8, (v >> 8) as u8, (v >> 16) as u8).hex()
    })
}

#[cfg(test)]
mod tests {
    use super::{Hit, LABEL_W, OPACITY_MIN, center_chip, hit, opacity_at, place_field, shortcut_chip, swatch, track, wheel_opacity};

    fn centre(r: crate::gfx::Rect) -> (f32, f32) {
        (r.x + r.w / 2.0, r.y + r.h / 2.0)
    }

    #[test]
    fn five_rows_fit_the_content_area() {
        // The rows themselves are checked at compile time, beside ROW_H. 146 is the content
        // area's height (`CONTENT` in app.rs).
        assert!(place_field().y + place_field().h <= 146.0);
        assert!(shortcut_chip().y + shortcut_chip().h < place_field().y);
    }

    #[test]
    fn hit_finds_each_row() {
        let at = |r| {
            let (x, y) = centre(r);
            hit(x, y)
        };
        assert!(matches!(at(swatch(0)), Some(Hit::Preset(_))));
        assert!(matches!(at(track()), Some(Hit::Opacity)));
        assert!(matches!(at(center_chip()), Some(Hit::Center)));
        assert!(matches!(at(shortcut_chip()), Some(Hit::Shortcut)));
        assert!(matches!(at(place_field()), Some(Hit::Place)));
        let between = (center_chip().y + center_chip().h + shortcut_chip().y) / 2.0;
        assert!(hit(LABEL_W + 10.0, between).is_none(), "between the Position and Shortcut chips");
    }

    #[test]
    fn slider_maps_its_length_to_the_opacity_range_in_steps() {
        let t = track();
        assert_eq!(opacity_at(t.x - 50.0), OPACITY_MIN);
        assert_eq!(opacity_at(t.x), OPACITY_MIN);
        assert_eq!(opacity_at(t.right()), 100);
        assert_eq!(opacity_at(t.right() + 50.0), 100);
        assert_eq!(opacity_at(t.x + t.w / 2.0) % 5, 0);
    }

    #[test]
    fn wheel_steps_opacity_only_over_the_slider() {
        let t = track();
        let (x, y) = (t.x + 10.0, t.y);
        assert_eq!(wheel_opacity(x, y, 50, 1.0), Some(55));
        assert_eq!(wheel_opacity(x, y, 50, -2.0), Some(40));
        assert_eq!(wheel_opacity(x, y, 100, 1.0), Some(100));
        assert_eq!(wheel_opacity(x, y, OPACITY_MIN, -1.0), Some(OPACITY_MIN));
        assert_eq!(wheel_opacity(x, y + 200.0, 50, 1.0), None);
    }
}
