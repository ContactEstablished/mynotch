//! Style tokens for the Halo glass look: colours, type, spacing and motion. Everything that
//! paints takes its values from here, so the look is tuned in one place.

use crate::gfx::Rgb;

/// Colours (opaque sRGB; translucency comes from `alpha`).
pub mod color {
    use super::Rgb;

    /// The glass tint, midnight blue; also the default body colour.
    pub const GLASS: Rgb = Rgb::from_hex(0x081624);
    pub const TEXT: Rgb = Rgb::from_hex(0xEAF7FF);
    pub const TEXT_DIM: Rgb = Rgb::from_hex(0xA8C3D5);
    /// The accent: progress, the active tab, the primary action.
    pub const CYAN: Rgb = Rgb::from_hex(0x91EAFF);
    /// The other half of the edge lighting.
    pub const VIOLET: Rgb = Rgb::from_hex(0xB3A2FB);
    /// Rest (timer breaks), beside the cyan of focus.
    pub const CALM: Rgb = Rgb::from_hex(0x7EE6C9);
    /// The sun, and lightning: a soft gold, the one warm colour.
    pub const SUN: Rgb = Rgb::from_hex(0xFFD27A);
    /// The moon's lit part.
    pub const MOON: Rgb = Rgb::from_hex(0xE4EEFA);
    /// Warnings, such as a shortcut another app owns.
    pub const WARN: Rgb = Rgb::from_hex(0xFFB05C);
    /// Recording: the dot and the glow while voice listens.
    pub const RECORD: Rgb = Rgb::from_hex(0xFF5D73);
    /// Behind selected text.
    pub const SELECTION: Rgb = Rgb::from_hex(0x24587A);
    /// The drop shadow, a blue-black rather than grey.
    pub const SHADOW: Rgb = Rgb::from_hex(0x01050A);
}

/// The timer styles' own colours (see `widgets::styles`). Halo uses the main palette.
pub mod styles {
    use super::Rgb;

    /// Pixel: hot pink blocks with a violet drop shadow, and a yellow block that blinks.
    pub const PIXEL: Rgb = Rgb::from_hex(0xFF7AC6);
    pub const PIXEL_SHADOW: Rgb = Rgb::from_hex(0x6A4BDB);
    pub const PIXEL_BLINK: Rgb = Rgb::from_hex(0xFFE45E);
    /// Dusk: a cream on a sunset running from amber through coral to violet.
    pub const DUSK_TEXT: Rgb = Rgb::from_hex(0xFFE6D0);
    pub const DUSK_DIM: Rgb = Rgb::from_hex(0xE2C2AE);
    pub const DUSK: [Rgb; 3] = [Rgb::from_hex(0xFFB869), Rgb::from_hex(0xFF7A8A), Rgb::from_hex(0xB39BFF)];
    /// Dots: lilac-white LEDs, and the violet of the edge lighting.
    pub const DOTS_TEXT: Rgb = Rgb::from_hex(0xECE7FF);
    pub const DOTS: Rgb = Rgb::from_hex(0xB3A2FB);
    /// Retro Clock: cream digits on charcoal flip tiles, and an amber dial.
    pub const RETRO: Rgb = Rgb::from_hex(0xFFB347);
    pub const RETRO_TEXT: Rgb = Rgb::from_hex(0xF4EAD5);
    pub const RETRO_TILE: Rgb = Rgb::from_hex(0x1B1E24);
    /// Terminal: phosphor green.
    pub const TERMINAL: Rgb = Rgb::from_hex(0x6CFF9A);
    pub const TERMINAL_DIM: Rgb = Rgb::from_hex(0x45B86C);
    /// Zen: sage, and a softer white.
    pub const ZEN: Rgb = Rgb::from_hex(0xA8E6CF);
    pub const ZEN_TEXT: Rgb = Rgb::from_hex(0xD8E6EE);
    /// Cyber HUD: electric cyan, marked in magenta.
    pub const HUD: Rgb = Rgb::from_hex(0x5CF2FF);
    pub const HUD_DIM: Rgb = Rgb::from_hex(0x7FC9D6);
    pub const HUD_MARK: Rgb = Rgb::from_hex(0xFF5CD6);
}

/// Opacities (0..=1) of the translucent inks.
pub mod alpha {
    /// White fills of controls at rest: chips, idle buttons, tracks.
    pub const TRACK: f32 = 0.07;
    /// The same under the cursor.
    pub const HOVER: f32 = 0.13;
    /// White hairlines and dividers.
    pub const LINE: f32 = 0.09;
    /// White outlines that should stand out.
    pub const EDGE: f32 = 0.20;
    /// Cyan fill of something lit: the active tab, the chosen chip, the primary button.
    pub const LIT: f32 = 0.13;
    /// Cyan outline of something lit.
    pub const LIT_EDGE: f32 = 0.36;
    /// The darker glass behind each card, so text stays readable over a bright wallpaper.
    pub const CARD_SHADE: f32 = 0.30;
    /// White wash over each card, lifting it off the body like a layer.
    pub const CARD_WASH: f32 = 0.035;
}

/// Type sizes in logical px. Big numbers are Rajdhani 500; everything else is Segoe UI
/// Variable (or Segoe UI).
pub mod text {
    /// The clock is the biggest number on the Time tab.
    pub const CLOCK: f32 = 48.0;
    pub const TIMER: f32 = 32.0;
    pub const STOPWATCH: f32 = 32.0;
    /// Seconds and tenths beside the big numbers.
    pub const FRACTION: f32 = 20.0;
    /// Small numerals: the collapsed rail.
    pub const NUMERAL: f32 = 15.0;
    /// The Terminal timer style's monospaced time, on a card and in the style gallery.
    pub const MONO: f32 = 26.0;
    pub const MONO_TILE: f32 = 17.0;
    /// Buttons, chips and the active tab's name.
    pub const CONTROL: f32 = 12.5;
    /// Uppercase section labels.
    pub const CAPTION: f32 = 11.0;
    /// Letter spacing of uppercase section labels.
    pub const TRACKING: f32 = 1.0;
}

/// Spacing and corner radii in logical px.
pub mod space {
    /// Between cards, and between a toolbar and what's under it.
    pub const GAP: f32 = 8.0;
    /// The panel's own padding.
    pub const PAD: f32 = 14.0;
    pub const CARD_PAD: f32 = 12.0;
    pub const BODY_RADIUS: f32 = 18.0;
    pub const CARD_RADIUS: f32 = 14.0;
    pub const CONTROL_RADIUS: f32 = 8.0;
    pub const TOOLBAR: f32 = 44.0;
}

/// Durations of transitions.
pub mod motion {
    use std::time::Duration;

    /// Hover in and out.
    pub const HOVER: Duration = Duration::from_millis(160);
    /// A state change: the active tab moving, a chip being chosen.
    pub const STATE: Duration = Duration::from_millis(190);
    /// The one soft pulse when a timer starts.
    pub const PULSE: Duration = Duration::from_millis(700);
    /// A Retro Clock digit flipping over.
    pub const FLIP: Duration = Duration::from_millis(340);
}
