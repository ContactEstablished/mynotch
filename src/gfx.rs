//! Drawing primitives shared by the shell and widgets. Widgets lay out in logical (96-DPI)
//! units relative to their own origin; `Canvas` maps that onto physical pixels. Colours,
//! sizes and timings come from `theme`.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use windows::{
    Win32::Graphics::{
        Direct2D::{Common::*, *},
        DirectWrite::*,
        Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM,
    },
    core::*,
};
use windows_numerics::{Matrix3x2, Vector2};

use crate::motion::{self, Motion};
use crate::theme::{alpha, color, motion as timing, text as type_size};

/// Decoded pixels (premultiplied BGRA, top-down) that any thread can produce and share as
/// `Arc<Image>`. The UI turns them into Direct2D bitmaps on demand (see `ImageCache`).
pub struct Image {
    id: u64,
    pub width: u32,
    pub height: u32,
    pixels: Vec<u8>,
}

impl Image {
    /// `pixels` must hold `width * height` premultiplied BGRA pixels.
    pub fn new(width: u32, height: u32, pixels: Vec<u8>) -> Option<Self> {
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        (width > 0 && height > 0 && pixels.len() == width as usize * height as usize * 4)
            .then(|| Self { id: NEXT_ID.fetch_add(1, Ordering::Relaxed), width, height, pixels })
    }

    #[cfg(test)]
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = (y * self.width + x) as usize * 4;
        [self.pixels[i], self.pixels[i + 1], self.pixels[i + 2], self.pixels[i + 3]]
    }
}

/// Identity: two `Image`s are equal only if they are the same decoded image.
impl PartialEq for Image {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl std::fmt::Debug for Image {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Image #{} ({}x{})", self.id, self.width, self.height)
    }
}

/// Direct2D bitmaps for `Image`s, keyed by image id. Device-dependent: cleared whenever the
/// render target is rebuilt.
#[derive(Default)]
pub struct ImageCache(RefCell<HashMap<u64, ID2D1Bitmap>>);

impl ImageCache {
    /// Generous for a dozen app icons, a screenful of clipboard thumbnails and album art;
    /// past it everything is dropped and re-uploaded as needed.
    const CAPACITY: usize = 128;

    pub fn clear(&self) {
        self.0.borrow_mut().clear();
    }
}

/// Logical-unit rectangle.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }

    /// Shrunk by `d` on every side (grown, for negative `d`).
    pub fn inset(&self, d: f32) -> Self {
        Self::new(self.x + d, self.y + d, self.w - d * 2.0, self.h - d * 2.0)
    }
}

/// An opaque sRGB colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    /// From `0xRRGGBB`.
    pub const fn from_hex(v: u32) -> Self {
        Self((v >> 16) as u8, (v >> 8) as u8, v as u8)
    }

    /// As `0xRRGGBB`.
    pub const fn hex(self) -> u32 {
        (self.0 as u32) << 16 | (self.1 as u32) << 8 | self.2 as u32
    }

    fn map(self, f: impl Fn(f32) -> f32) -> Self {
        let c = |v: u8| f(f32::from(v)).round().clamp(0.0, 255.0) as u8;
        Self(c(self.0), c(self.1), c(self.2))
    }

    /// Mixed a fraction `k` of the way to white.
    fn lighten(self, k: f32) -> Self {
        self.map(|v| v + (255.0 - v) * k)
    }

    /// Each channel scaled by `k`.
    fn darken(self, k: f32) -> Self {
        self.map(|v| v * k)
    }

    fn with_alpha(self, a: f32) -> D2D1_COLOR_F {
        rgba(self.0, self.1, self.2, a)
    }
}

/// How the notch body is painted.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    pub color: Rgb,
    /// 0..=1; scales the body and its shadow, not the content.
    pub opacity: f32,
}

/// Content colours. The translucent ones read as glass set into the body rather than as
/// flat paint.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ink {
    Text,
    Dim,
    /// Faint white fill of controls at rest.
    Track,
    /// Brighter white fill, under the cursor.
    Hover,
    /// Cyan: progress and the primary action.
    Accent,
    /// The violet half of the edge lighting.
    Violet,
    /// Teal, for rest (timer breaks).
    Calm,
    /// Faint white, for hairlines and outlines.
    Line,
    /// Brighter white, for outlines that should stand out.
    Edge,
    /// Cyan glass: the fill of something lit (the active tab, the chosen chip).
    Lit,
    /// The outline of something lit.
    LitEdge,
    /// Any opaque colour (colour swatches, badges).
    Rgb(Rgb),
}

const fn rgba(r: u8, g: u8, b: u8, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r: r as f32 / 255.0, g: g as f32 / 255.0, b: b as f32 / 255.0, a }
}

const WHITE: Rgb = Rgb(255, 255, 255);

unsafe fn gradient_stops(rt: &ID2D1DCRenderTarget, colors: &[(f32, D2D1_COLOR_F)]) -> Result<ID2D1GradientStopCollection> {
    let stops: Vec<D2D1_GRADIENT_STOP> =
        colors.iter().map(|&(position, color)| D2D1_GRADIENT_STOP { position, color }).collect();
    unsafe { rt.CreateGradientStopCollection(&stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP) }
}

unsafe fn linear(rt: &ID2D1DCRenderTarget, colors: &[(f32, D2D1_COLOR_F)]) -> Result<ID2D1LinearGradientBrush> {
    unsafe {
        rt.CreateLinearGradientBrush(&D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES::default(), None, &gradient_stops(rt, colors)?)
    }
}

unsafe fn radial(rt: &ID2D1DCRenderTarget, colors: &[(f32, D2D1_COLOR_F)]) -> Result<ID2D1RadialGradientBrush> {
    unsafe {
        rt.CreateRadialGradientBrush(&D2D1_RADIAL_GRADIENT_BRUSH_PROPERTIES::default(), None, &gradient_stops(rt, colors)?)
    }
}

/// The glass body material (see `Canvas::glass`): a translucent tint in the user's colour,
/// lit along its edges in cyan and violet. Gradient geometry follows the animating notch,
/// so it is set on every draw.
struct Glass {
    /// The tint, a little lighter at the top and deeper at the bottom.
    base: ID2D1LinearGradientBrush,
    /// Soft white reflection from the upper left.
    sheen: ID2D1RadialGradientBrush,
    /// Faint cyan bloom near the top left and violet bloom in the lower right.
    cyan: ID2D1RadialGradientBrush,
    violet: ID2D1RadialGradientBrush,
    /// Light falling a little way into the glass from its top edge.
    fall: ID2D1LinearGradientBrush,
    /// The hairline around the edge: cyan on the left into violet on the right.
    rim: ID2D1LinearGradientBrush,
    /// The illuminated top edge, fading out towards the corners.
    top: ID2D1LinearGradientBrush,
    /// A second, fainter hairline just inside the rim, for the glass's thickness.
    inner: ID2D1SolidColorBrush,
    shadow: ID2D1SolidColorBrush,
    /// Body opacity, which the shadow follows.
    opacity: f32,
}

impl Glass {
    unsafe fn new(rt: &ID2D1DCRenderTarget, look: Look) -> Result<Self> {
        let (tint, o) = (look.color, look.opacity.clamp(0.0, 1.0));
        let (cyan, violet) = (color::CYAN, color::VIOLET);
        unsafe {
            Ok(Self {
                base: linear(
                    rt,
                    &[
                        // A little denser than the opacity says, so text stays readable over a
                        // bright wallpaper; deepest at the bottom.
                        (0.0, tint.lighten(0.06).with_alpha((o * 1.1).min(1.0))),
                        (0.6, tint.with_alpha((o * 1.12).min(1.0))),
                        (1.0, tint.darken(0.7).with_alpha((o * 1.18).min(1.0))),
                    ],
                )?,
                sheen: radial(rt, &[(0.0, WHITE.with_alpha(0.06 * o)), (1.0, WHITE.with_alpha(0.0))])?,
                cyan: radial(rt, &[(0.0, cyan.with_alpha(0.09 * o)), (1.0, cyan.with_alpha(0.0))])?,
                violet: radial(rt, &[(0.0, violet.with_alpha(0.10 * o)), (1.0, violet.with_alpha(0.0))])?,
                fall: linear(rt, &[(0.0, cyan.with_alpha(0.08)), (1.0, cyan.with_alpha(0.0))])?,
                rim: linear(
                    rt,
                    &[(0.0, cyan.with_alpha(0.30)), (0.5, WHITE.with_alpha(0.13)), (1.0, violet.with_alpha(0.30))],
                )?,
                top: linear(
                    rt,
                    &[
                        (0.0, cyan.with_alpha(0.0)),
                        (0.22, cyan.with_alpha(0.85)),
                        (0.5, color::TEXT.with_alpha(0.65)),
                        (0.78, violet.with_alpha(0.8)),
                        (1.0, violet.with_alpha(0.0)),
                    ],
                )?,
                inner: rt.CreateSolidColorBrush(&WHITE.with_alpha(0.035), None)?,
                shadow: rt.CreateSolidColorBrush(&color::SHADOW.with_alpha(1.0), None)?,
                opacity: o,
            })
        }
    }
}

/// Device-dependent brushes; rebuilt together with the render target.
pub struct Brushes {
    text: ID2D1SolidColorBrush,
    dim: ID2D1SolidColorBrush,
    track: ID2D1SolidColorBrush,
    hover: ID2D1SolidColorBrush,
    accent: ID2D1SolidColorBrush,
    violet: ID2D1SolidColorBrush,
    calm: ID2D1SolidColorBrush,
    line: ID2D1SolidColorBrush,
    edge: ID2D1SolidColorBrush,
    lit: ID2D1SolidColorBrush,
    lit_edge: ID2D1SolidColorBrush,
    /// Recoloured for each `Ink::Rgb` use.
    paint: ID2D1SolidColorBrush,
    /// The darker glass behind cards, in the body's colour.
    shade: ID2D1SolidColorBrush,
    /// Cyan into violet: the timer's ring.
    sweep: ID2D1LinearGradientBrush,
    /// Cyan, brightest in the middle and fading at both ends: the active tab's underline.
    beam: ID2D1LinearGradientBrush,
    /// White fading downwards: light catching a card's top edge.
    catch: ID2D1LinearGradientBrush,
    /// Round caps, for arcs and glowing lines.
    round: ID2D1StrokeStyle,
    /// Gradients through colours other than the sweep's (the timer styles'), made on first
    /// use and keyed by their colours.
    gradients: RefCell<HashMap<Vec<u32>, ID2D1LinearGradientBrush>>,
    glass: Glass,
    /// Last value given to `set_content_opacity`, applied to images too.
    content_opacity: Cell<f32>,
}

impl Brushes {
    pub unsafe fn new(rt: &ID2D1DCRenderTarget, look: Look) -> Result<Self> {
        let solid = |c: D2D1_COLOR_F| unsafe { rt.CreateSolidColorBrush(&c, None) };
        let (cyan, violet) = (color::CYAN, color::VIOLET);
        unsafe {
            let caps = D2D1_STROKE_STYLE_PROPERTIES {
                startCap: D2D1_CAP_STYLE_ROUND,
                endCap: D2D1_CAP_STYLE_ROUND,
                dashCap: D2D1_CAP_STYLE_ROUND,
                lineJoin: D2D1_LINE_JOIN_ROUND,
                miterLimit: 10.0,
                dashStyle: D2D1_DASH_STYLE_SOLID,
                dashOffset: 0.0,
            };
            Ok(Self {
                text: solid(color::TEXT.with_alpha(1.0))?,
                dim: solid(color::TEXT_DIM.with_alpha(1.0))?,
                track: solid(WHITE.with_alpha(alpha::TRACK))?,
                hover: solid(WHITE.with_alpha(alpha::HOVER))?,
                accent: solid(cyan.with_alpha(1.0))?,
                violet: solid(violet.with_alpha(1.0))?,
                calm: solid(color::CALM.with_alpha(1.0))?,
                line: solid(WHITE.with_alpha(alpha::LINE))?,
                edge: solid(WHITE.with_alpha(alpha::EDGE))?,
                lit: solid(cyan.with_alpha(alpha::LIT))?,
                lit_edge: solid(cyan.with_alpha(alpha::LIT_EDGE))?,
                paint: solid(rgba(0, 0, 0, 1.0))?,
                shade: solid(shade_color(look))?,
                sweep: linear(rt, &[(0.0, cyan.with_alpha(1.0)), (1.0, violet.with_alpha(1.0))])?,
                beam: linear(
                    rt,
                    &[(0.0, cyan.with_alpha(0.0)), (0.5, cyan.with_alpha(1.0)), (1.0, cyan.with_alpha(0.0))],
                )?,
                catch: linear(rt, &[(0.0, WHITE.with_alpha(0.16)), (1.0, WHITE.with_alpha(0.0))])?,
                round: rt.GetFactory()?.CreateStrokeStyle(&caps, None)?,
                gradients: RefCell::default(),
                glass: Glass::new(rt, look)?,
                content_opacity: Cell::new(1.0),
            })
        }
    }

    /// Repaints the body in a new colour or opacity.
    pub unsafe fn restyle(&mut self, rt: &ID2D1DCRenderTarget, look: Look) -> Result<()> {
        self.glass = unsafe { Glass::new(rt, look)? };
        unsafe { self.shade.SetColor(&shade_color(look)) };
        Ok(())
    }

    fn get(&self, ink: Ink) -> &ID2D1SolidColorBrush {
        match ink {
            Ink::Text => &self.text,
            Ink::Dim => &self.dim,
            Ink::Track => &self.track,
            Ink::Hover => &self.hover,
            Ink::Accent => &self.accent,
            Ink::Violet => &self.violet,
            Ink::Calm => &self.calm,
            Ink::Line => &self.line,
            Ink::Edge => &self.edge,
            Ink::Lit => &self.lit,
            Ink::LitEdge => &self.lit_edge,
            Ink::Rgb(c) => {
                unsafe { self.paint.SetColor(&c.with_alpha(1.0)) };
                &self.paint
            }
        }
    }

    /// Every brush that paints content (not the body), which fades in and out with it.
    fn content(&self) -> [&ID2D1Brush; 17] {
        [
            &self.text,
            &self.dim,
            &self.track,
            &self.hover,
            &self.accent,
            &self.violet,
            &self.calm,
            &self.line,
            &self.edge,
            &self.lit,
            &self.lit_edge,
            &self.paint,
            &self.shade,
            &self.sweep,
            &self.beam,
            &self.catch,
            &self.glass.inner,
        ]
    }

    /// Opacity of every content ink (not the body), used to fade content in and out.
    pub fn set_content_opacity(&self, a: f32) {
        self.content_opacity.set(a);
        for b in self.content() {
            unsafe { b.SetOpacity(a) };
        }
    }

    /// Runs `draw` with `brush` at a fraction `a` of the content opacity.
    fn faded<T>(&self, brush: &ID2D1Brush, a: f32, draw: impl FnOnce() -> T) -> T {
        let base = self.content_opacity.get();
        unsafe { brush.SetOpacity(base * a.clamp(0.0, 1.0)) };
        let out = draw();
        unsafe { brush.SetOpacity(base) };
        out
    }
}

/// The darker glass behind cards: the body's colour, deepened.
fn shade_color(look: Look) -> D2D1_COLOR_F {
    look.color.darken(0.55).with_alpha(alpha::CARD_SHADE)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Font {
    /// 12px regular: secondary text.
    Small,
    /// 11px semibold: uppercase section labels (see `Canvas::caps`).
    Caption,
    /// 12.5px semibold: buttons, chips, the active tab's name.
    Label,
    /// 15px semibold.
    Title,
    /// 13px regular.
    Body,
    /// 18px semibold.
    Heading,
    /// 16px icon glyphs.
    Icon,
    /// 10px icon glyphs, for badges.
    IconSmall,
    /// 13px icon glyphs, beside section labels.
    IconCaption,
    /// 20px icon glyphs.
    IconMedium,
    /// 24px icon glyphs.
    IconLarge,
    /// Rajdhani 500, 32px: the timer.
    Timer,
    /// Rajdhani 500, 48px: the clock.
    Clock,
    /// Rajdhani 500, 32px: the stopwatch, and the timer on Home.
    Stopwatch,
    /// Rajdhani 500, 20px: seconds and tenths beside the big numbers.
    Fraction,
    /// Rajdhani 500, 15px: the collapsed rail and the session counter.
    Numeral,
    /// Cascadia Mono (or Consolas), 26px: the Terminal timer style's time.
    MonoTimer,
    /// The same, 17px, in the style gallery.
    MonoTile,
    /// The same, 12px: the Terminal style's status line and progress bar.
    MonoSmall,
}

#[derive(Clone, Copy, PartialEq)]
enum Family {
    Ui,
    Icons,
    Numerals,
    Mono,
}

impl Font {
    /// In declaration order, so `font as usize` indexes it.
    const ALL: [Font; 19] = [
        Font::Small,
        Font::Caption,
        Font::Label,
        Font::Title,
        Font::Body,
        Font::Heading,
        Font::Icon,
        Font::IconSmall,
        Font::IconCaption,
        Font::IconMedium,
        Font::IconLarge,
        Font::Timer,
        Font::Clock,
        Font::Stopwatch,
        Font::Fraction,
        Font::Numeral,
        Font::MonoTimer,
        Font::MonoTile,
        Font::MonoSmall,
    ];

    fn spec(self) -> (Family, f32, DWRITE_FONT_WEIGHT) {
        use Family::*;
        let (normal, semibold) = (DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_WEIGHT_SEMI_BOLD);
        match self {
            Font::Small => (Ui, 12.0, normal),
            Font::Caption => (Ui, type_size::CAPTION, semibold),
            Font::Label => (Ui, type_size::CONTROL, semibold),
            Font::Title => (Ui, 15.0, semibold),
            Font::Body => (Ui, 13.0, normal),
            Font::Heading => (Ui, 18.0, semibold),
            Font::Icon => (Icons, 16.0, normal),
            Font::IconSmall => (Icons, 10.0, normal),
            Font::IconCaption => (Icons, 13.0, normal),
            Font::IconMedium => (Icons, 20.0, normal),
            Font::IconLarge => (Icons, 24.0, normal),
            Font::Timer => (Numerals, type_size::TIMER, DWRITE_FONT_WEIGHT_MEDIUM),
            Font::Clock => (Numerals, type_size::CLOCK, DWRITE_FONT_WEIGHT_MEDIUM),
            Font::Stopwatch => (Numerals, type_size::STOPWATCH, DWRITE_FONT_WEIGHT_MEDIUM),
            Font::Fraction => (Numerals, type_size::FRACTION, DWRITE_FONT_WEIGHT_MEDIUM),
            Font::Numeral => (Numerals, type_size::NUMERAL, DWRITE_FONT_WEIGHT_MEDIUM),
            Font::MonoTimer => (Mono, type_size::MONO, normal),
            Font::MonoTile => (Mono, type_size::MONO_TILE, normal),
            Font::MonoSmall => (Mono, 12.0, normal),
        }
    }
}

/// Glyphs from Segoe Fluent Icons (Windows 11) / Segoe MDL2 Assets (Windows 10); the
/// code points are shared by both fonts.
pub mod glyph {
    pub const PREVIOUS: &str = "\u{E892}";
    pub const NEXT: &str = "\u{E893}";
    pub const PLAY: &str = "\u{E768}";
    pub const PAUSE: &str = "\u{E769}";
    pub const MUSIC: &str = "\u{E8D6}";
    pub const PIN: &str = "\u{E718}";
    pub const PINNED: &str = "\u{E842}";
    pub const RESET: &str = "\u{E72C}";
    pub const APPS: &str = "\u{E71D}";
    pub const CLIPBOARD: &str = "\u{E77F}";
    pub const DOCUMENT: &str = "\u{E8A5}";
    pub const LINK: &str = "\u{E71B}";
    pub const DELETE: &str = "\u{E74D}";
    pub const HOME: &str = "\u{E80F}";
    pub const CLOCK: &str = "\u{E917}";
    pub const TILES: &str = "\u{E74C}";
    pub const PULSE: &str = "\u{E9D9}";
    pub const GEAR: &str = "\u{E713}";
    pub const CLOSE: &str = "\u{E894}";
    pub const CHECK: &str = "\u{E73E}";
    pub const PALETTE: &str = "\u{E790}";
    pub const SEARCH: &str = "\u{E721}";
    pub const STOPWATCH: &str = "\u{E916}";
    pub const EYE: &str = "\u{E890}";
}

/// The numerals face, Rajdhani 500, bundled into the executable.
pub struct Numerals {
    collection: IDWriteFontCollection,
    family: HSTRING,
}

impl Numerals {
    /// Loads the bundled font into a private collection. `None` where DirectWrite is too old
    /// for in-memory fonts (before Windows 10 1703); the numbers then use the UI font.
    pub fn load(dwrite: &IDWriteFactory) -> Option<Self> {
        static DATA: &[u8] = include_bytes!("../assets/fonts/Rajdhani-Medium.ttf");
        unsafe {
            let factory: IDWriteFactory5 = dwrite.cast().ok()?;
            let loader = factory.CreateInMemoryFontFileLoader().ok()?;
            factory.RegisterFontFileLoader(&loader).ok()?;
            // No owner: the loader keeps its own copy of the data.
            let file =
                loader.CreateInMemoryFontFileReference(&factory, DATA.as_ptr().cast(), DATA.len() as u32, None).ok()?;
            let builder: IDWriteFontSetBuilder1 = factory.CreateFontSetBuilder().ok()?.cast().ok()?;
            builder.AddFontFile(&file).ok()?;
            let collection: IDWriteFontCollection = factory.CreateFontCollectionFromFontSet(&builder.CreateFontSet().ok()?).ok()?.cast().ok()?;
            // Grouped by weight/width/style the family is "Rajdhani"; some versions of the
            // name table only offer "Rajdhani Medium".
            let family = ["Rajdhani", "Rajdhani Medium"].into_iter().map(HSTRING::from).find(|name| has_family(&collection, name))?;
            Some(Self { collection, family })
        }
    }
}

fn has_family(collection: &IDWriteFontCollection, name: &HSTRING) -> bool {
    let (mut index, mut exists) = (0, BOOL(0));
    unsafe { collection.FindFamilyName(name, &mut index, &mut exists).is_ok() && exists.as_bool() }
}

fn system_has(dwrite: &IDWriteFactory, name: &HSTRING) -> bool {
    let mut fonts = None;
    let found = unsafe { dwrite.GetSystemFontCollection(&mut fonts, false) }.is_ok();
    found && fonts.is_some_and(|fonts| has_family(&fonts, name))
}

/// Single-line text formats (no wrapping, ellipsis on overflow), sized for one DPI scale.
pub struct Fonts {
    /// Kept for measuring text.
    dwrite: IDWriteFactory,
    scale: f32,
    /// Indexed by `Font as usize`.
    formats: Vec<IDWriteTextFormat>,
    /// Advance of a char in a font (digits all get the widest digit's), in logical px.
    advances: RefCell<HashMap<(Font, char), f32>>,
    /// Baseline and line height of a font, in logical px.
    lines: RefCell<HashMap<Font, (f32, f32)>>,
}

impl Fonts {
    pub unsafe fn new(dwrite: &IDWriteFactory, numerals: Option<&Numerals>, scale: f32) -> Result<Self> {
        // Segoe UI Variable on Windows 11; Segoe UI on Windows 10.
        let variable = HSTRING::from("Segoe UI Variable Text");
        let ui = if system_has(dwrite, &variable) { variable } else { HSTRING::from("Segoe UI") };
        let fluent = HSTRING::from("Segoe Fluent Icons");
        let icons = if system_has(dwrite, &fluent) { fluent } else { HSTRING::from("Segoe MDL2 Assets") };
        // Cascadia Mono ships with Windows 11; Consolas with every version since Vista.
        let cascadia = HSTRING::from("Cascadia Mono");
        let mono = if system_has(dwrite, &cascadia) { cascadia } else { HSTRING::from("Consolas") };
        let formats = Font::ALL
            .iter()
            .map(|&font| {
                let (family, size, weight) = font.spec();
                let (name, collection, weight) = match (family, numerals) {
                    (Family::Ui, _) => (&ui, None, weight),
                    (Family::Icons, _) => (&icons, None, weight),
                    (Family::Mono, _) => (&mono, None, weight),
                    (Family::Numerals, Some(n)) => (&n.family, Some(&n.collection), weight),
                    // Without the bundled face, a light UI face keeps big numbers airy.
                    (Family::Numerals, None) => (&ui, None, DWRITE_FONT_WEIGHT_SEMI_LIGHT),
                };
                unsafe { text_format(dwrite, name, collection, weight, size * scale) }
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            dwrite: dwrite.clone(),
            scale,
            formats,
            advances: RefCell::default(),
            lines: RefCell::default(),
        })
    }

    fn get(&self, font: Font) -> &IDWriteTextFormat {
        &self.formats[font as usize]
    }

    /// A one-line layout of `s`, its width unbounded.
    fn layout(&self, s: &str, font: Font) -> Option<IDWriteTextLayout> {
        let wide: Vec<u16> = s.encode_utf16().collect();
        let layout = unsafe { self.dwrite.CreateTextLayout(&wide, self.get(font), f32::MAX, f32::MAX) }.ok()?;
        // The format is shared, so it carries whatever alignment `text` last drew with;
        // centred or trailing in an f32::MAX-wide box, the width is lost to rounding.
        unsafe {
            let _ = layout.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_LEADING);
        }
        Some(layout)
    }

    /// Width of `s` set on one line, in logical px.
    fn width(&self, s: &str, font: Font) -> f32 {
        let mut metrics = DWRITE_TEXT_METRICS::default();
        match self.layout(s, font).map(|l| unsafe { l.GetMetrics(&mut metrics) }) {
            Some(Ok(())) => metrics.widthIncludingTrailingWhitespace / self.scale,
            _ => 0.0,
        }
    }

    /// How far `c` advances in `font`. Every digit advances as far as the widest, which keeps
    /// changing numbers still (the bundled face has no tabular figures of its own).
    fn advance(&self, font: Font, c: char) -> f32 {
        let key = (font, if c.is_ascii_digit() { '0' } else { c });
        if let Some(&w) = self.advances.borrow().get(&key) {
            return w;
        }
        let w = if c.is_ascii_digit() {
            ('0'..='9').map(|d| self.width(d.encode_utf8(&mut [0; 4]), font)).fold(0.0, f32::max)
        } else {
            self.width(c.encode_utf8(&mut [0; 4]), font)
        };
        self.advances.borrow_mut().insert(key, w);
        w
    }

    /// Baseline (from the top of the line) and line height of `font`.
    fn line(&self, font: Font) -> (f32, f32) {
        if let Some(&line) = self.lines.borrow().get(&font) {
            return line;
        }
        let mut metrics = [DWRITE_LINE_METRICS::default()];
        let mut count = 0;
        let line = match self.layout("0", font).map(|l| unsafe { l.GetLineMetrics(Some(&mut metrics), &mut count) }) {
            Some(Ok(())) => (metrics[0].baseline / self.scale, metrics[0].height / self.scale),
            _ => (0.0, 0.0),
        };
        self.lines.borrow_mut().insert(font, line);
        line
    }
}

unsafe fn text_format(
    dwrite: &IDWriteFactory,
    family: &HSTRING,
    collection: Option<&IDWriteFontCollection>,
    weight: DWRITE_FONT_WEIGHT,
    size: f32,
) -> Result<IDWriteTextFormat> {
    unsafe {
        let fmt = dwrite.CreateTextFormat(
            family,
            collection,
            weight,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            size,
            w!("en-us"),
        )?;
        fmt.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
        fmt.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
        let trimming =
            DWRITE_TRIMMING { granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER, delimiter: 0, delimiterCount: 0 };
        let sign = dwrite.CreateEllipsisTrimmingSign(&fmt)?;
        fmt.SetTrimming(&trimming, &sign)?;
        Ok(fmt)
    }
}

/// Lines and elliptical arcs in logical units, built up like a pen's moves; made into a
/// `Shape` by `Canvas::path_shape`.
#[derive(Clone, Debug, Default)]
pub struct Path(Vec<Seg>);

#[derive(Clone, Copy, Debug)]
enum Seg {
    Move(f32, f32),
    Line(f32, f32),
    Arc { x: f32, y: f32, rx: f32, ry: f32, large: bool, clockwise: bool },
    Close,
}

impl Path {
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts a new figure at (`x`, `y`).
    pub fn move_to(mut self, x: f32, y: f32) -> Self {
        self.0.push(Seg::Move(x, y));
        self
    }

    pub fn line_to(mut self, x: f32, y: f32) -> Self {
        self.0.push(Seg::Line(x, y));
        self
    }

    /// An elliptical arc to (`x`, `y`). Clockwise is as seen on screen (y down).
    pub fn arc_to(mut self, x: f32, y: f32, (rx, ry): (f32, f32), large: bool, clockwise: bool) -> Self {
        self.0.push(Seg::Arc { x, y, rx, ry, large, clockwise });
        self
    }

    pub fn close(mut self) -> Self {
        self.0.push(Seg::Close);
        self
    }
}

/// A shape on one canvas (in its physical pixels), to fill or stroke there.
pub struct Shape(ID2D1Geometry);

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Align {
    Left,
    Center,
    Right,
}

fn text_alignment(align: Align) -> DWRITE_TEXT_ALIGNMENT {
    match align {
        Align::Left => DWRITE_TEXT_ALIGNMENT_LEADING,
        Align::Center => DWRITE_TEXT_ALIGNMENT_CENTER,
        Align::Right => DWRITE_TEXT_ALIGNMENT_TRAILING,
    }
}

/// A drawing surface in logical units. Cheap to copy; `offset` makes a child canvas whose
/// origin is shifted, so a widget can draw from (0, 0).
#[derive(Clone, Copy)]
pub struct Canvas<'a> {
    rt: &'a ID2D1DCRenderTarget,
    brushes: &'a Brushes,
    fonts: &'a Fonts,
    images: &'a ImageCache,
    motion: &'a Motion,
    scale: f32,
    /// Physical position of logical (0, 0).
    ox: f32,
    oy: f32,
    /// Cursor in this canvas's logical coordinates, when it is over the expanded notch.
    cursor: Option<(f32, f32)>,
}

impl<'a> Canvas<'a> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        rt: &'a ID2D1DCRenderTarget,
        brushes: &'a Brushes,
        fonts: &'a Fonts,
        images: &'a ImageCache,
        motion: &'a Motion,
        scale: f32,
        origin: (f32, f32),
        cursor: Option<(f32, f32)>,
    ) -> Self {
        Self { rt, brushes, fonts, images, motion, scale, ox: origin.0, oy: origin.1, cursor }
    }

    pub fn offset(&self, dx: f32, dy: f32) -> Self {
        Self {
            ox: self.ox + dx * self.scale,
            oy: self.oy + dy * self.scale,
            cursor: self.cursor.map(|(x, y)| (x - dx, y - dy)),
            ..*self
        }
    }

    pub fn hovered(&self, r: Rect) -> bool {
        self.cursor.is_some_and(|(x, y)| r.contains(x, y))
    }

    /// How hovered `r` is, 0..=1, easing in and out over `theme::motion::HOVER`.
    pub fn hover(&self, r: Rect) -> f32 {
        self.hover_if(r, true)
    }

    /// `hover`, for a control that only lights up while `enabled`.
    pub fn hover_if(&self, r: Rect, enabled: bool) -> f32 {
        let px = self.px(r);
        let key = motion::key(("hover", [px.left, px.top, px.right, px.bottom].map(|v| v.round() as i32)));
        self.motion.ease(key, if enabled && self.hovered(r) { 1.0 } else { 0.0 }, timing::HOVER)
    }

    /// `key`'s value, easing towards `target` over `over` (see `Motion::ease`).
    pub fn ease(&self, key: u64, target: f32, over: Duration) -> f32 {
        self.motion.ease(key, target, over)
    }

    fn px(&self, r: Rect) -> D2D_RECT_F {
        let s = self.scale;
        D2D_RECT_F {
            left: self.ox + r.x * s,
            top: self.oy + r.y * s,
            right: self.ox + (r.x + r.w) * s,
            bottom: self.oy + (r.y + r.h) * s,
        }
    }

    fn point(&self, x: f32, y: f32) -> Vector2 {
        Vector2 { X: self.ox + x * self.scale, Y: self.oy + y * self.scale }
    }

    fn rounded(&self, r: Rect, radius: f32) -> D2D1_ROUNDED_RECT {
        D2D1_ROUNDED_RECT { rect: self.px(r), radiusX: radius * self.scale, radiusY: radius * self.scale }
    }

    /// One line of text, vertically centred in `r`, trimmed with an ellipsis if too long.
    pub fn text(&self, s: &str, font: Font, ink: Ink, r: Rect, align: Align) {
        let fmt = self.fonts.get(font);
        let wide: Vec<u16> = s.encode_utf16().collect();
        unsafe {
            let _ = fmt.SetTextAlignment(text_alignment(align));
            self.rt.DrawText(
                &wide,
                fmt,
                &self.px(r),
                self.brushes.get(ink),
                D2D1_DRAW_TEXT_OPTIONS_CLIP,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        }
    }

    /// `text` at a fraction `a` of its ink's opacity; drawn over the same text in another
    /// ink, it cross-fades between the two.
    pub fn text_a(&self, s: &str, font: Font, ink: Ink, r: Rect, align: Align, a: f32) {
        if a > 0.0 {
            self.brushes.faded(self.brushes.get(ink), a, || self.text(s, font, ink, r, align));
        }
    }

    /// A small uppercase label with open letter spacing, vertically centred in `r`.
    pub fn caps(&self, s: &str, font: Font, ink: Ink, r: Rect, align: Align) {
        let wide: Vec<u16> = s.to_uppercase().encode_utf16().collect();
        let px = self.px(r);
        unsafe {
            let Ok(layout) =
                self.fonts.dwrite.CreateTextLayout(&wide, self.fonts.get(font), px.right - px.left, px.bottom - px.top)
            else {
                return;
            };
            let _ = layout.SetTextAlignment(text_alignment(align));
            if let Ok(spaced) = layout.cast::<IDWriteTextLayout1>() {
                let all = DWRITE_TEXT_RANGE { startPosition: 0, length: wide.len() as u32 };
                let _ = spaced.SetCharacterSpacing(0.0, type_size::TRACKING * self.scale, 0.0, all);
            }
            self.rt.DrawTextLayout(
                Vector2 { X: px.left, Y: px.top },
                &layout,
                self.brushes.get(ink),
                D2D1_DRAW_TEXT_OPTIONS_CLIP,
            );
        }
    }

    /// One line of text from `x`, standing on a baseline `baseline` px from the top. (In a
    /// monospaced font this sets digits just as `numerals` does, in one call.)
    pub fn text_at(&self, s: &str, font: Font, ink: Ink, x: f32, baseline: f32) {
        let (base, height) = self.fonts.line(font);
        let w = self.measure(s, font) + 4.0;
        self.text(s, font, ink, Rect::new(x, baseline - base, w, height), Align::Left);
    }

    /// Width of `s` set on one line in `font`.
    pub fn measure(&self, s: &str, font: Font) -> f32 {
        self.fonts.width(s, font)
    }

    /// Width of `runs` as `numerals` sets them.
    pub fn numerals_width(&self, runs: &[(&str, Font, Ink)]) -> f32 {
        runs.iter().flat_map(|&(s, font, _)| s.chars().map(move |c| (font, c))).map(|(f, c)| self.fonts.advance(f, c)).sum()
    }

    /// Numbers in runs of (text, font, ink), sharing one baseline `baseline` px from the top,
    /// starting at `x` (or ending there, or centred on it). Digits sit in equal cells, so a
    /// changing number doesn't shuffle sideways. Returns the width.
    pub fn numerals(&self, runs: &[(&str, Font, Ink)], x: f32, baseline: f32, align: Align) -> f32 {
        let width = self.numerals_width(runs);
        let mut at = match align {
            Align::Left => x,
            Align::Center => x - width / 2.0,
            Align::Right => x - width,
        };
        for &(s, font, ink) in runs {
            let (base, height) = self.fonts.line(font);
            for c in s.chars() {
                let advance = self.fonts.advance(font, c);
                // Centred in its cell; the slack keeps the ellipsis away.
                let cell = Rect::new(at - 10.0, baseline - base, advance + 20.0, height);
                self.text(c.encode_utf8(&mut [0; 4]), font, ink, cell, Align::Center);
                at += advance;
            }
        }
        width
    }

    pub fn fill(&self, r: Rect, radius: f32, ink: Ink) {
        unsafe { self.rt.FillRoundedRectangle(&self.rounded(r, radius), self.brushes.get(ink)) };
    }

    /// `fill` at a fraction `a` of the ink's opacity.
    pub fn fill_a(&self, r: Rect, radius: f32, ink: Ink, a: f32) {
        if a > 0.0 {
            self.brushes.faded(self.brushes.get(ink), a, || self.fill(r, radius, ink));
        }
    }

    /// A 1px outline just inside `r`, at a fraction `a` of the ink's opacity.
    pub fn outline(&self, r: Rect, radius: f32, ink: Ink, a: f32) {
        if a <= 0.0 {
            return;
        }
        let rr = self.rounded(r.inset(0.5), (radius - 0.5).max(0.0));
        let brush = self.brushes.get(ink);
        self.brushes.faded(brush, a, || unsafe { self.rt.DrawRoundedRectangle(&rr, brush, self.scale, None) });
    }

    /// A 1px line.
    pub fn line(&self, (x0, y0): (f32, f32), (x1, y1): (f32, f32), ink: Ink) {
        unsafe { self.rt.DrawLine(self.point(x0, y0), self.point(x1, y1), self.brushes.get(ink), self.scale, None) };
    }

    /// Filled circle, or an outline of `stroke` logical px.
    pub fn circle(&self, cx: f32, cy: f32, radius: f32, ink: Ink, stroke: Option<f32>) {
        let s = self.scale;
        let e = D2D1_ELLIPSE { point: self.point(cx, cy), radiusX: radius * s, radiusY: radius * s };
        unsafe {
            match stroke {
                Some(w) => self.rt.DrawEllipse(&e, self.brushes.get(ink), w * s, None),
                None => self.rt.FillEllipse(&e, self.brushes.get(ink)),
            }
        }
    }

    /// `circle` at a fraction `a` of the ink's opacity.
    pub fn circle_a(&self, cx: f32, cy: f32, radius: f32, ink: Ink, stroke: Option<f32>, a: f32) {
        if a > 0.0 {
            self.brushes.faded(self.brushes.get(ink), a, || self.circle(cx, cy, radius, ink, stroke));
        }
    }

    /// A soft glow around a circle of `radius`: faint rings fading outward. `strength` 0..=1.
    pub fn halo(&self, cx: f32, cy: f32, radius: f32, ink: Ink, strength: f32) {
        for (grow, width, a) in [(1.5, 2.5, 0.16), (3.5, 3.0, 0.08), (6.5, 4.0, 0.035)] {
            self.circle_a(cx, cy, radius + grow, ink, Some(width), a * strength);
        }
    }

    /// A ring `width` px wide whose stroke runs clockwise from 12 o'clock for `fraction` of a
    /// turn, in `ink`, or in the cyan-to-violet sweep for `None`; `a` scales its opacity.
    #[allow(clippy::too_many_arguments)]
    pub fn ring(&self, cx: f32, cy: f32, radius: f32, width: f32, fraction: f32, ink: Option<Ink>, a: f32) {
        let brush: &ID2D1Brush = match ink {
            Some(ink) => self.brushes.get(ink),
            None => {
                let sweep = &self.brushes.sweep;
                self.diagonal(sweep, cx, cy, radius);
                sweep
            }
        };
        self.arc(cx, cy, radius, width, fraction, brush, a);
    }

    /// `ring` in a gradient through `colors`, from its top left to its bottom right.
    #[allow(clippy::too_many_arguments)]
    pub fn ring_gradient(&self, cx: f32, cy: f32, radius: f32, width: f32, fraction: f32, colors: &[Rgb], a: f32) {
        if let Some(brush) = self.gradient(colors) {
            self.diagonal(&brush, cx, cy, radius);
            self.arc(cx, cy, radius, width, fraction, &brush, a);
        }
    }

    /// Lays `brush` across a circle's bounds, from its top left to its bottom right.
    fn diagonal(&self, brush: &ID2D1LinearGradientBrush, cx: f32, cy: f32, radius: f32) {
        let (c, r) = (self.point(cx, cy), radius * self.scale);
        unsafe {
            brush.SetStartPoint(Vector2 { X: c.X - r, Y: c.Y - r });
            brush.SetEndPoint(Vector2 { X: c.X + r, Y: c.Y + r });
        }
    }

    /// A linear gradient through `colors`, evenly spaced; each set is made once.
    fn gradient(&self, colors: &[Rgb]) -> Option<ID2D1LinearGradientBrush> {
        let key: Vec<u32> = colors.iter().map(|c| c.hex()).collect();
        if let Some(brush) = self.brushes.gradients.borrow().get(&key) {
            return Some(brush.clone());
        }
        let last = colors.len().saturating_sub(1).max(1) as f32;
        let stops: Vec<_> = colors.iter().enumerate().map(|(i, c)| (i as f32 / last, c.with_alpha(1.0))).collect();
        let brush = unsafe { linear(self.rt, &stops) }.ok()?;
        self.brushes.gradients.borrow_mut().insert(key, brush.clone());
        Some(brush)
    }

    /// `fill` in a gradient through `colors`, running from x = `from` to x = `to` (so a
    /// progress bar's colour at its end follows where the end is).
    pub fn fill_gradient(&self, r: Rect, radius: f32, colors: &[Rgb], (from, to): (f32, f32), a: f32) {
        let Some(brush) = self.gradient(colors).filter(|_| a > 0.0) else { return };
        let (start, end) = (self.point(from, r.y), self.point(to, r.y));
        unsafe {
            brush.SetStartPoint(start);
            brush.SetEndPoint(end);
        }
        let rr = self.rounded(r, radius);
        self.brushes.faded(&brush, a, || unsafe { self.rt.FillRoundedRectangle(&rr, &brush) });
    }

    /// Runs `draw` with `ink` at a fraction `a` of its opacity, for drawing that doesn't take
    /// one (e.g. `numerals`). Everything `draw` paints should be in `ink`.
    pub fn faded(&self, ink: Ink, a: f32, draw: impl FnOnce()) {
        if a > 0.0 {
            self.brushes.faded(self.brushes.get(ink), a, draw);
        }
    }

    /// Runs `draw` squashed vertically by `k` (1 = as drawn, 0 = flat) towards the line
    /// y = `y`: a flap seen turning on a hinge there.
    pub fn squashed(&self, y: f32, k: f32, draw: impl FnOnce()) {
        let mut old = Matrix3x2::identity();
        unsafe { self.rt.GetTransform(&mut old) };
        let hinge = Matrix3x2::scale_around(1.0, k.max(0.001), self.point(0.0, y));
        unsafe { self.rt.SetTransform(&(hinge * old)) };
        draw();
        unsafe { self.rt.SetTransform(&old) };
    }

    /// The stroke of `ring`, in `brush`.
    #[allow(clippy::too_many_arguments)]
    fn arc(&self, cx: f32, cy: f32, radius: f32, width: f32, fraction: f32, brush: &ID2D1Brush, a: f32) {
        let f = fraction.clamp(0.0, 1.0);
        if f <= 0.0 || a <= 0.0 {
            return;
        }
        let s = self.scale;
        let (c, r) = (self.point(cx, cy), radius * s);
        self.brushes.faded(brush, a, || unsafe {
            if f >= 0.999 {
                let e = D2D1_ELLIPSE { point: c, radiusX: r, radiusY: r };
                self.rt.DrawEllipse(&e, brush, width * s, None);
                return;
            }
            let Some(path) = self.rt.GetFactory().ok().and_then(|f| f.CreatePathGeometry().ok()) else { return };
            let Ok(sink) = path.Open() else { return };
            let turn = f * std::f32::consts::TAU;
            sink.BeginFigure(Vector2 { X: c.X, Y: c.Y - r }, D2D1_FIGURE_BEGIN_HOLLOW);
            sink.AddArc(&D2D1_ARC_SEGMENT {
                point: Vector2 { X: c.X + r * turn.sin(), Y: c.Y - r * turn.cos() },
                size: D2D_SIZE_F { width: r, height: r },
                rotationAngle: 0.0,
                sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                arcSize: if f > 0.5 { D2D1_ARC_SIZE_LARGE } else { D2D1_ARC_SIZE_SMALL },
            });
            sink.EndFigure(D2D1_FIGURE_END_OPEN);
            if sink.Close().is_ok() {
                self.rt.DrawGeometry(&path, brush, width * s, &self.brushes.round);
            }
        });
    }

    /// A line `width` px wide with round ends, at a fraction `a` of the ink's opacity.
    #[allow(clippy::too_many_arguments)]
    pub fn stroke_line(&self, (x0, y0): (f32, f32), (x1, y1): (f32, f32), ink: Ink, width: f32, a: f32) {
        let brush = self.brushes.get(ink);
        self.brushes.faded(brush, a, || unsafe {
            self.rt.DrawLine(self.point(x0, y0), self.point(x1, y1), brush, width * self.scale, &self.brushes.round);
        });
    }

    /// `path` as a shape on this canvas.
    pub fn path_shape(&self, path: &Path) -> Option<Shape> {
        let s = self.scale;
        unsafe {
            let geometry = self.rt.GetFactory().ok()?.CreatePathGeometry().ok()?;
            let sink = geometry.Open().ok()?;
            let mut open = false;
            for seg in &path.0 {
                match *seg {
                    Seg::Move(x, y) => {
                        if open {
                            sink.EndFigure(D2D1_FIGURE_END_OPEN);
                        }
                        sink.BeginFigure(self.point(x, y), D2D1_FIGURE_BEGIN_FILLED);
                        open = true;
                    }
                    Seg::Line(x, y) => sink.AddLine(self.point(x, y)),
                    Seg::Arc { x, y, rx, ry, large, clockwise } => sink.AddArc(&D2D1_ARC_SEGMENT {
                        point: self.point(x, y),
                        size: D2D_SIZE_F { width: rx * s, height: ry * s },
                        rotationAngle: 0.0,
                        sweepDirection: if clockwise {
                            D2D1_SWEEP_DIRECTION_CLOCKWISE
                        } else {
                            D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE
                        },
                        arcSize: if large { D2D1_ARC_SIZE_LARGE } else { D2D1_ARC_SIZE_SMALL },
                    }),
                    Seg::Close => {
                        sink.EndFigure(D2D1_FIGURE_END_CLOSED);
                        open = false;
                    }
                }
            }
            if open {
                sink.EndFigure(D2D1_FIGURE_END_OPEN);
            }
            sink.Close().ok()?;
            geometry.cast().ok().map(Shape)
        }
    }

    /// The union of some circles (cx, cy, r) and rectangles: e.g. a cloud's outline.
    pub fn blob(&self, circles: &[(f32, f32, f32)], rects: &[Rect]) -> Option<Shape> {
        let s = self.scale;
        unsafe {
            let factory = self.rt.GetFactory().ok()?;
            let mut parts: Vec<ID2D1Geometry> = Vec::new();
            for &(cx, cy, r) in circles {
                let e = D2D1_ELLIPSE { point: self.point(cx, cy), radiusX: r * s, radiusY: r * s };
                parts.push(factory.CreateEllipseGeometry(&e).ok()?.cast().ok()?);
            }
            for &r in rects {
                let rr = D2D1_ROUNDED_RECT { rect: self.px(r), radiusX: 0.0, radiusY: 0.0 };
                parts.push(factory.CreateRoundedRectangleGeometry(&rr).ok()?.cast().ok()?);
            }
            let mut parts = parts.into_iter();
            let first = Shape(parts.next()?);
            parts.try_fold(first, |acc, part| self.combine(&acc, &Shape(part), D2D1_COMBINE_MODE_UNION))
        }
    }

    /// `a` with `b` cut out of it.
    pub fn minus(&self, a: &Shape, b: &Shape) -> Option<Shape> {
        self.combine(a, b, D2D1_COMBINE_MODE_EXCLUDE)
    }

    fn combine(&self, a: &Shape, b: &Shape, mode: D2D1_COMBINE_MODE) -> Option<Shape> {
        unsafe {
            let out = self.rt.GetFactory().ok()?.CreatePathGeometry().ok()?;
            let sink = out.Open().ok()?;
            a.0.CombineWithGeometry(&b.0, mode, None, D2D1_DEFAULT_FLATTENING_TOLERANCE, &sink).ok()?;
            sink.Close().ok()?;
            out.cast().ok().map(Shape)
        }
    }

    /// Runs `draw` with its drawing confined to `mask`.
    pub fn masked(&self, mask: &Shape, draw: impl FnOnce()) {
        let params = D2D1_LAYER_PARAMETERS {
            contentBounds: D2D_RECT_F { left: -f32::MAX, top: -f32::MAX, right: f32::MAX, bottom: f32::MAX },
            geometricMask: std::mem::ManuallyDrop::new(Some(mask.0.clone())),
            maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
            maskTransform: Matrix3x2::identity(),
            opacity: 1.0,
            opacityBrush: std::mem::ManuallyDrop::new(None),
            layerOptions: D2D1_LAYER_OPTIONS_NONE,
        };
        unsafe { self.rt.PushLayer(&params, None) };
        draw();
        unsafe { self.rt.PopLayer() };
        drop(std::mem::ManuallyDrop::into_inner(params.geometricMask));
    }

    /// Everything except `shape` (within a generous box around the canvas origin), as a mask
    /// for `masked`.
    pub fn outside(&self, shape: &Shape) -> Option<Shape> {
        let everything = self.blob(&[], &[Rect::new(-4000.0, -4000.0, 8000.0, 8000.0)])?;
        self.minus(&everything, shape)
    }

    pub fn fill_shape(&self, shape: &Shape, ink: Ink, a: f32) {
        let brush = self.brushes.get(ink);
        self.brushes.faded(brush, a, || unsafe { self.rt.FillGeometry(&shape.0, brush, None) });
    }

    pub fn stroke_shape(&self, shape: &Shape, ink: Ink, width: f32, a: f32) {
        let brush = self.brushes.get(ink);
        self.brushes.faded(brush, a, || unsafe {
            self.rt.DrawGeometry(&shape.0, brush, width * self.scale, &self.brushes.round);
        });
    }

    /// A short horizontal line of cyan light at `y`, brightest in the middle, with a soft
    /// glow around it: the active tab's underline. `strength` 0..=1.
    pub fn beam(&self, x0: f32, x1: f32, y: f32, strength: f32) {
        if strength <= 0.0 || x1 <= x0 {
            return;
        }
        let (a, b) = (self.point(x0, y), self.point(x1, y));
        let beam = &self.brushes.beam;
        unsafe {
            beam.SetStartPoint(a);
            beam.SetEndPoint(b);
        }
        for (width, k) in [(7.0, 0.14), (3.5, 0.3), (1.5, 1.0)] {
            self.brushes.faded(beam, k * strength, || unsafe {
                self.rt.DrawLine(a, b, beam, width * self.scale, &self.brushes.round);
            });
        }
    }

    /// A card: a darker, slightly lifted layer of the glass, with a hairline edge that
    /// catches the light along its top. `lit` (0..=1) edges it in cyan with a soft glow, for
    /// the card that matters most.
    pub fn card(&self, r: Rect, radius: f32, lit: f32) {
        self.card_in(r, radius, lit, color::CYAN);
    }

    /// `card`, lit in `accent` instead of cyan.
    pub fn card_in(&self, r: Rect, radius: f32, lit: f32, accent: Rgb) {
        let rr = self.rounded(r, radius);
        let px = rr.rect;
        unsafe {
            self.rt.FillRoundedRectangle(&rr, &self.brushes.shade);
        }
        self.fill_a(r, radius, Ink::Track, alpha::CARD_WASH / alpha::TRACK);
        self.fill_a(r, radius, Ink::Rgb(accent), alpha::LIT * 0.4 * lit);
        self.outline(r, radius, Ink::Line, 1.0);
        let catch = &self.brushes.catch;
        unsafe {
            catch.SetStartPoint(Vector2 { X: px.left, Y: px.top });
            catch.SetEndPoint(Vector2 { X: px.left, Y: px.top + (px.bottom - px.top) * 0.5 });
            let inset = 0.5 * self.scale;
            let edge = D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F { left: px.left + inset, top: px.top + inset, right: px.right - inset, bottom: px.bottom - inset },
                radiusX: rr.radiusX - inset,
                radiusY: rr.radiusY - inset,
            };
            self.rt.DrawRoundedRectangle(&edge, catch, self.scale, None);
        }
        if lit > 0.0 {
            self.outline(r, radius, Ink::Rgb(accent), alpha::LIT_EDGE * 0.75 * lit);
            for (grow, width, a) in [(1.5, 2.0, 0.10), (3.5, 3.0, 0.04)] {
                let glow = self.rounded(r.inset(-grow), radius + grow);
                let brush = self.brushes.get(Ink::Rgb(accent));
                self.brushes.faded(brush, a * lit, || unsafe {
                    self.rt.DrawRoundedRectangle(&glow, brush, width * self.scale, None);
                });
            }
        }
    }

    /// A comet of light running clockwise around the edge of a rounded rect: its bright head
    /// is `head` (0..1) of the way round, its tail fading behind, `stretch` times its usual
    /// length. Shows something is running.
    #[allow(clippy::too_many_arguments)]
    pub fn runner(&self, r: Rect, radius: f32, head: f32, ink: Ink, a: f32, stretch: f32) {
        if a <= 0.0 {
            return;
        }
        let s = self.scale;
        let rr = self.rounded(r.inset(0.5), (radius - 0.5).max(0.0));
        let (w, h, rad) = (rr.rect.right - rr.rect.left, rr.rect.bottom - rr.rect.top, rr.radiusX);
        let perimeter = 2.0 * (w + h) - 8.0 * rad + std::f32::consts::TAU * rad;
        let Ok(factory) = (unsafe { self.rt.GetFactory() }) else { return };
        let brush = self.brushes.get(ink);
        // Overlapping dashes that all end at the head: long and faint, short and bright, and
        // a wide soft glow.
        for (length, width, alpha) in [(0.26, 1.2, 0.25), (0.14, 1.4, 0.55), (0.05, 1.8, 1.0), (0.12, 5.0, 0.12)] {
            let (len, width) = ((perimeter * length * stretch).clamp(width * s, perimeter * 0.6), width * s);
            let start = (head.rem_euclid(1.0) * perimeter - len).rem_euclid(perimeter);
            // Dashes are measured in stroke widths: one dash, then a gap for the rest of the way.
            let props = D2D1_STROKE_STYLE_PROPERTIES {
                startCap: D2D1_CAP_STYLE_ROUND,
                endCap: D2D1_CAP_STYLE_ROUND,
                dashCap: D2D1_CAP_STYLE_ROUND,
                lineJoin: D2D1_LINE_JOIN_ROUND,
                miterLimit: 10.0,
                dashStyle: D2D1_DASH_STYLE_CUSTOM,
                dashOffset: -start / width,
            };
            let dashes = [len / width, (perimeter - len) / width];
            let Ok(style) = (unsafe { factory.CreateStrokeStyle(&props, Some(&dashes)) }) else { return };
            self.brushes.faded(brush, alpha * a, || unsafe { self.rt.DrawRoundedRectangle(&rr, brush, width, &style) });
        }
    }

    /// The notch body in glass. `r` may extend above y = 0 (to keep the top corners
    /// off-screen); gradients span only its visible part. `open` (0..1) is how far the notch
    /// has expanded: it scales a soft drop shadow, which must stay 0 while collapsed since
    /// shadow pixels would catch clicks, and brightens the lit top edge.
    pub fn glass(&self, r: Rect, radius: f32, open: f32) {
        let g = &self.brushes.glass;
        let s = self.scale;
        let px = self.px(r);
        let top = px.top.max(self.oy);
        let (w, h) = (px.right - px.left, px.bottom - top);
        let rounded = |rect: D2D_RECT_F, radius: f32| D2D1_ROUNDED_RECT { rect, radiusX: radius, radiusY: radius };
        let body = rounded(px, radius * s);

        unsafe {
            // Concentric, increasingly faint layers approximate a blurred shadow.
            if open > 0.0 {
                const LAYERS: usize = 12;
                for i in (1..=LAYERS).rev() {
                    let grow = i as f32 * 1.4 * s;
                    let k = 1.0 - i as f32 / (LAYERS + 1) as f32;
                    g.shadow.SetOpacity(0.07 * k * k * open * g.opacity.max(0.5));
                    let rect = D2D_RECT_F {
                        left: px.left - grow,
                        top: px.top,
                        right: px.right + grow,
                        bottom: px.bottom + grow * 1.3,
                    };
                    self.rt.FillRoundedRectangle(&rounded(rect, radius * s + grow), &g.shadow);
                }
            }

            g.base.SetStartPoint(Vector2 { X: px.left, Y: top });
            g.base.SetEndPoint(Vector2 { X: px.left, Y: px.bottom });
            self.rt.FillRoundedRectangle(&body, &g.base);

            g.sheen.SetCenter(Vector2 { X: px.left + w * 0.25, Y: top });
            g.sheen.SetRadiusX(w * 0.6);
            g.sheen.SetRadiusY(h);
            self.rt.FillRoundedRectangle(&body, &g.sheen);

            g.cyan.SetCenter(Vector2 { X: px.left + w * 0.15, Y: top });
            g.cyan.SetRadiusX(w * 0.35);
            g.cyan.SetRadiusY(h * 0.8);
            self.rt.FillRoundedRectangle(&body, &g.cyan);

            g.violet.SetCenter(Vector2 { X: px.left + w * 0.88, Y: px.bottom });
            g.violet.SetRadiusX(w * 0.45);
            g.violet.SetRadiusY(h * 1.1);
            self.rt.FillRoundedRectangle(&body, &g.violet);

            g.fall.SetStartPoint(Vector2 { X: px.left, Y: top });
            g.fall.SetEndPoint(Vector2 { X: px.left, Y: top + (28.0 * s).min(h * 0.5) });
            self.rt.FillRoundedRectangle(&body, &g.fall);

            // Hairlines centred half a pixel inside the edge, then a pixel further in.
            let edge = |inset: f32| {
                let rect = D2D_RECT_F {
                    left: px.left + inset,
                    top: px.top + inset,
                    right: px.right - inset,
                    bottom: px.bottom - inset,
                };
                rounded(rect, radius * s - inset)
            };
            g.rim.SetStartPoint(Vector2 { X: px.left, Y: top });
            g.rim.SetEndPoint(Vector2 { X: px.right, Y: top });
            self.rt.DrawRoundedRectangle(&edge(0.5 * s), &g.rim, s, None);
            self.rt.DrawRoundedRectangle(&edge(1.5 * s), &g.inner, s, None);

            // The lit top edge, along the very top of the body.
            let (x0, x1) = (px.left + radius * s, px.right - radius * s);
            if x1 > x0 {
                g.top.SetStartPoint(Vector2 { X: x0, Y: top });
                g.top.SetEndPoint(Vector2 { X: x1, Y: top });
                let y = top + 0.5 * s;
                for (width, k) in [(1.0, 1.0), (4.0, 0.18)] {
                    g.top.SetOpacity(k * (0.35 + 0.65 * open));
                    self.rt.DrawLine(Vector2 { X: x0, Y: y }, Vector2 { X: x1, Y: y }, &g.top, width * s, None);
                }
            }
        }
    }

    /// Draws `image` to fill `r` (cropping to keep its aspect ratio), with rounded corners.
    pub fn image(&self, image: &Image, r: Rect, radius: f32) {
        let mut cache = self.images.0.borrow_mut();
        if !cache.contains_key(&image.id) {
            if cache.len() >= ImageCache::CAPACITY {
                cache.clear();
            }
            let props = D2D1_BITMAP_PROPERTIES {
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: 96.0,
                dpiY: 96.0,
            };
            let size = D2D_SIZE_U { width: image.width, height: image.height };
            let pixels = Some(image.pixels.as_ptr().cast());
            match unsafe { self.rt.CreateBitmap(size, pixels, image.width * 4, &props) } {
                Ok(bitmap) => cache.insert(image.id, bitmap),
                Err(_) => return,
            };
        }
        let Some(bitmap) = cache.get(&image.id) else { return };
        let dest = self.px(r);
        let (dw, dh) = (dest.right - dest.left, dest.bottom - dest.top);
        // "Cover" fit: scale to fill, centred.
        let k = (dw / image.width as f32).max(dh / image.height as f32);
        let transform = Matrix3x2 {
            M11: k,
            M12: 0.0,
            M21: 0.0,
            M22: k,
            M31: dest.left + (dw - image.width as f32 * k) / 2.0,
            M32: dest.top + (dh - image.height as f32 * k) / 2.0,
        };
        let props = D2D1_BITMAP_BRUSH_PROPERTIES {
            extendModeX: D2D1_EXTEND_MODE_CLAMP,
            extendModeY: D2D1_EXTEND_MODE_CLAMP,
            interpolationMode: D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
        };
        unsafe {
            let Ok(brush) = self.rt.CreateBitmapBrush(bitmap, Some(&props), None) else { return };
            brush.SetTransform(&transform);
            brush.SetOpacity(self.brushes.content_opacity.get());
            let rr = D2D1_ROUNDED_RECT { rect: dest, radiusX: radius * self.scale, radiusY: radius * self.scale };
            self.rt.FillRoundedRectangle(&rr, &brush);
        }
    }

    /// Restricts drawing to `r` until `pop_clip`.
    pub fn push_clip(&self, r: Rect) {
        unsafe { self.rt.PushAxisAlignedClip(&self.px(r), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE) };
    }

    pub fn pop_clip(&self) {
        unsafe { self.rt.PopAxisAlignedClip() };
    }
}

#[cfg(test)]
mod tests {
    use super::{Font, Rgb};
    use crate::settings::DEFAULT_COLOR;
    use crate::theme::color;

    #[test]
    fn hex_round_trips() {
        assert_eq!(Rgb::from_hex(0x12AB3C), Rgb(0x12, 0xAB, 0x3C));
        assert_eq!(Rgb(0x12, 0xAB, 0x3C).hex(), 0x12AB3C);
    }

    #[test]
    fn default_body_is_the_glass_tint() {
        assert_eq!(Rgb::from_hex(DEFAULT_COLOR), color::GLASS);
    }

    #[test]
    fn fonts_are_indexed_by_declaration_order() {
        for (i, font) in Font::ALL.iter().enumerate() {
            assert_eq!(*font as usize, i, "{font:?}");
        }
    }

    #[test]
    fn lighten_and_darken() {
        assert_eq!(Rgb(0, 100, 200).lighten(0.5), Rgb(128, 178, 228));
        assert_eq!(Rgb(0, 100, 200).darken(0.5), Rgb(0, 50, 100));
    }
}
