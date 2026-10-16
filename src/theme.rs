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
    /// Behind selected text.
    pub const SELECTION: Rgb = Rgb::from_hex(0x24587A);
    /// The drop shadow, a blue-black rather than grey.
    pub const SHADOW: Rgb = Rgb::from_hex(0x01050A);
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
    pub const FOCUS: f32 = 48.0;
    pub const CLOCK: f32 = 40.0;
    pub const STOPWATCH: f32 = 32.0;
    /// Seconds and tenths beside the big numbers.
    pub const FRACTION: f32 = 20.0;
    /// Small numerals: the collapsed rail and the session counter.
    pub const NUMERAL: f32 = 15.0;
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
}
