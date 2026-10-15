//! Drawing primitives shared by the shell and widgets. Widgets lay out in logical (96-DPI)
//! units relative to their own origin; `Canvas` maps that onto physical pixels.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use windows::{
    Win32::Graphics::{
        Direct2D::{Common::*, *},
        DirectWrite::*,
        Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM,
    },
    core::*,
};
use windows_numerics::{Matrix3x2, Vector2};

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

    /// A bright, fairly saturated colour of the same hue, for the body's inner glow. Greys
    /// glow plain white.
    fn glow(self) -> Self {
        let [r, g, b] = [self.0, self.1, self.2].map(|v| f32::from(v) / 255.0);
        let (max, min) = (r.max(g).max(b), r.min(g).min(b));
        let d = max - min;
        if d < 0.02 {
            return Self(230, 230, 236);
        }
        // Hue in sixths of a turn.
        let h = if max == r {
            ((g - b) / d).rem_euclid(6.0)
        } else if max == g {
            (b - r) / d + 2.0
        } else {
            (r - g) / d + 4.0
        };
        let (v, s) = (0.9, (d / max).max(0.6));
        let c = v * s;
        let x = c * (1.0 - (h.rem_euclid(2.0) - 1.0).abs());
        let (r, g, b) = match h as u32 {
            0 => (c, x, 0.0),
            1 => (x, c, 0.0),
            2 => (0.0, c, x),
            3 => (0.0, x, c),
            4 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };
        let m = v - c;
        let f =|v: f32| ((v + m) * 255.0).round() as u8;
        Self(f(r), f(g), f(b))
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

/// Content colours. `Track` and `Hover` are translucent white, so controls read as glass
/// set into the body rather than flat grey.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ink {
    Text,
    Dim,
    Track,
    Hover,
    Accent,
    /// Teal, for rest (timer breaks).
    Calm,
    /// Faint translucent white, for outlines.
    Line,
    /// Brighter translucent white, for outlines that should stand out.
    Edge,
    /// Any opaque colour (colour swatches, badges).
    Rgb(Rgb),
}

const fn rgba(r: u8, g: u8, b: u8, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r: r as f32 / 255.0, g: g as f32 / 255.0, b: b as f32 / 255.0, a }
}

/// The glass body material (see `Canvas::glass`), in the user's colour. Gradient geometry
/// follows the animating notch, so it is set on every draw.
struct Glass {
    /// The body colour, slightly see-through; lifts a touch towards the top.
    base: ID2D1LinearGradientBrush,
    /// Soft reflection of light from the upper left.
    sheen: ID2D1RadialGradientBrush,
    /// Faint glow of the body's hue in the lower right (violet for obsidian).
    tint: ID2D1RadialGradientBrush,
    /// Light catching the edge, strongest along the bottom.
    rim: ID2D1LinearGradientBrush,
    shadow: ID2D1SolidColorBrush,
    /// Body opacity, which the shadow follows.
    opacity: f32,
}

impl Glass {
    unsafe fn new(rt: &ID2D1DCRenderTarget, look: Look) -> Result<Self> {
        let (color, o) = (look.color, look.opacity.clamp(0.0, 1.0));
        let glow = color.glow();
        unsafe {
            let stops = |colors: &[(f32, D2D1_COLOR_F)]| {
                let stops: Vec<D2D1_GRADIENT_STOP> =
                    colors.iter().map(|&(position, color)| D2D1_GRADIENT_STOP { position, color }).collect();
                rt.CreateGradientStopCollection(&stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP)
            };
            let linear = |colors: &[(f32, D2D1_COLOR_F)]| -> Result<ID2D1LinearGradientBrush> {
                rt.CreateLinearGradientBrush(&D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES::default(), None, &stops(colors)?)
            };
            let radial = |colors: &[(f32, D2D1_COLOR_F)]| -> Result<ID2D1RadialGradientBrush> {
                rt.CreateRadialGradientBrush(&D2D1_RADIAL_GRADIENT_BRUSH_PROPERTIES::default(), None, &stops(colors)?)
            };
            // The default colour gives the original obsidian: top (18, 16, 25), bottom (4, 4, 7).
            Ok(Self {
                base: linear(&[
                    (0.0, color.lighten(0.03).with_alpha(0.96 * o)),
                    (1.0, color.darken(0.4).with_alpha(0.97 * o)),
                ])?,
                sheen: radial(&[(0.0, rgba(255, 255, 255, 0.10 * o)), (1.0, rgba(255, 255, 255, 0.0))])?,
                tint: radial(&[(0.0, glow.with_alpha(0.14 * o)), (1.0, glow.with_alpha(0.0))])?,
                rim: linear(&[(0.0, rgba(255, 255, 255, 0.04)), (1.0, rgba(255, 255, 255, 0.28))])?,
                shadow: rt.CreateSolidColorBrush(&rgba(0, 0, 0, 1.0), None)?,
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
    calm: ID2D1SolidColorBrush,
    line: ID2D1SolidColorBrush,
    edge: ID2D1SolidColorBrush,
    /// Recoloured for each `Ink::Rgb` use.
    paint: ID2D1SolidColorBrush,
    glass: Glass,
    /// Last value given to `set_content_opacity`, applied to images too.
    content_opacity: Cell<f32>,
}

impl Brushes {
    pub unsafe fn new(rt: &ID2D1DCRenderTarget, look: Look) -> Result<Self> {
        let solid = |c: D2D1_COLOR_F| unsafe { rt.CreateSolidColorBrush(&c, None) };
        Ok(Self {
            text: solid(rgba(242, 242, 247, 1.0))?,
            dim: solid(rgba(158, 160, 176, 1.0))?,
            track: solid(rgba(255, 255, 255, 0.09))?,
            hover: solid(rgba(255, 255, 255, 0.16))?,
            accent: solid(rgba(122, 176, 255, 1.0))?,
            calm: solid(rgba(102, 214, 184, 1.0))?,
            line: solid(rgba(255, 255, 255, 0.12))?,
            edge: solid(rgba(255, 255, 255, 0.30))?,
            paint: solid(rgba(0, 0, 0, 1.0))?,
            glass: unsafe { Glass::new(rt, look)? },
            content_opacity: Cell::new(1.0),
        })
    }

    /// Repaints the body in a new colour or opacity.
    pub unsafe fn restyle(&mut self, rt: &ID2D1DCRenderTarget, look: Look) -> Result<()> {
        self.glass = unsafe { Glass::new(rt, look)? };
        Ok(())
    }

    fn get(&self, ink: Ink) -> &ID2D1SolidColorBrush {
        match ink {
            Ink::Text => &self.text,
            Ink::Dim => &self.dim,
            Ink::Track => &self.track,
            Ink::Hover => &self.hover,
            Ink::Accent => &self.accent,
            Ink::Calm => &self.calm,
            Ink::Line => &self.line,
            Ink::Edge => &self.edge,
            Ink::Rgb(c) => {
                unsafe { self.paint.SetColor(&c.with_alpha(1.0)) };
                &self.paint
            }
        }
    }

    /// Opacity of every content ink (not the body), used to fade content in and out.
    pub fn set_content_opacity(&self, a: f32) {
        self.content_opacity.set(a);
        unsafe {
            for b in [
                &self.text,
                &self.dim,
                &self.track,
                &self.hover,
                &self.accent,
                &self.calm,
                &self.line,
                &self.edge,
                &self.paint,
            ] {
                b.SetOpacity(a);
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Font {
    /// 12px regular.
    Small,
    /// 13px semibold.
    Label,
    /// 15px semibold.
    Title,
    /// 16px icon glyphs.
    Icon,
    /// 20px icon glyphs.
    IconMedium,
    /// 24px icon glyphs.
    IconLarge,
    /// 32px light, for big numbers.
    Display,
    /// 13px regular.
    Body,
    /// 18px semibold.
    Heading,
    /// 10px icon glyphs, for badges.
    IconSmall,
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
}

/// Single-line text formats (no wrapping, ellipsis on overflow), sized for one DPI scale.
pub struct Fonts {
    /// Kept for measuring text.
    dwrite: IDWriteFactory,
    small: IDWriteTextFormat,
    label: IDWriteTextFormat,
    title: IDWriteTextFormat,
    icon: IDWriteTextFormat,
    icon_medium: IDWriteTextFormat,
    icon_large: IDWriteTextFormat,
    display: IDWriteTextFormat,
    body: IDWriteTextFormat,
    heading: IDWriteTextFormat,
    icon_small: IDWriteTextFormat,
}

impl Fonts {
    pub unsafe fn new(dwrite: &IDWriteFactory, scale: f32) -> Result<Self> {
        unsafe {
            let icons = icon_family(dwrite);
            let make = |family: PCWSTR, size: f32, weight: DWRITE_FONT_WEIGHT| -> Result<IDWriteTextFormat> {
                let fmt = dwrite.CreateTextFormat(
                    family,
                    None,
                    weight,
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    size * scale,
                    w!("en-us"),
                )?;
                fmt.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
                fmt.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
                let trimming = DWRITE_TRIMMING {
                    granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
                    delimiter: 0,
                    delimiterCount: 0,
                };
                let sign = dwrite.CreateEllipsisTrimmingSign(&fmt)?;
                fmt.SetTrimming(&trimming, &sign)?;
                Ok(fmt)
            };
            let ui = w!("Segoe UI");
            Ok(Self {
                dwrite: dwrite.clone(),
                small: make(ui, 12.0, DWRITE_FONT_WEIGHT_NORMAL)?,
                label: make(ui, 13.0, DWRITE_FONT_WEIGHT_SEMI_BOLD)?,
                title: make(ui, 15.0, DWRITE_FONT_WEIGHT_SEMI_BOLD)?,
                icon: make(icons, 16.0, DWRITE_FONT_WEIGHT_NORMAL)?,
                icon_medium: make(icons, 20.0, DWRITE_FONT_WEIGHT_NORMAL)?,
                icon_large: make(icons, 24.0, DWRITE_FONT_WEIGHT_NORMAL)?,
                display: make(ui, 32.0, DWRITE_FONT_WEIGHT_LIGHT)?,
                body: make(ui, 13.0, DWRITE_FONT_WEIGHT_NORMAL)?,
                heading: make(ui, 18.0, DWRITE_FONT_WEIGHT_SEMI_BOLD)?,
                icon_small: make(icons, 10.0, DWRITE_FONT_WEIGHT_NORMAL)?,
            })
        }
    }

    fn get(&self, font: Font) -> &IDWriteTextFormat {
        match font {
            Font::Small => &self.small,
            Font::Label => &self.label,
            Font::Title => &self.title,
            Font::Icon => &self.icon,
            Font::IconMedium => &self.icon_medium,
            Font::IconLarge => &self.icon_large,
            Font::Display => &self.display,
            Font::Body => &self.body,
            Font::Heading => &self.heading,
            Font::IconSmall => &self.icon_small,
        }
    }
}

/// Segoe Fluent Icons where installed (Windows 11), else Segoe MDL2 Assets (Windows 10).
unsafe fn icon_family(dwrite: &IDWriteFactory) -> PCWSTR {
    let fluent = w!("Segoe Fluent Icons");
    unsafe {
        let mut fonts = None;
        if dwrite.GetSystemFontCollection(&mut fonts, false).is_ok()
            && let Some(fonts) = fonts
        {
            let (mut index, mut exists) = (0, BOOL(0));
            if fonts.FindFamilyName(fluent, &mut index, &mut exists).is_ok() && exists.as_bool() {
                return fluent;
            }
        }
    }
    w!("Segoe MDL2 Assets")
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Align {
    Left,
    Center,
    Right,
}

/// A drawing surface in logical units. Cheap to copy; `offset` makes a child canvas whose
/// origin is shifted, so a widget can draw from (0, 0).
#[derive(Clone, Copy)]
pub struct Canvas<'a> {
    rt: &'a ID2D1DCRenderTarget,
    brushes: &'a Brushes,
    fonts: &'a Fonts,
    images: &'a ImageCache,
    scale: f32,
    /// Physical position of logical (0, 0).
    ox: f32,
    oy: f32,
    /// Cursor in this canvas's logical coordinates, when it is over the expanded notch.
    cursor: Option<(f32, f32)>,
}

impl<'a> Canvas<'a> {
    pub fn new(
        rt: &'a ID2D1DCRenderTarget,
        brushes: &'a Brushes,
        fonts: &'a Fonts,
        images: &'a ImageCache,
        scale: f32,
        origin: (f32, f32),
        cursor: Option<(f32, f32)>,
    ) -> Self {
        Self { rt, brushes, fonts, images, scale, ox: origin.0, oy: origin.1, cursor }
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

    fn px(&self, r: Rect) -> D2D_RECT_F {
        let s = self.scale;
        D2D_RECT_F {
            left: self.ox + r.x * s,
            top: self.oy + r.y * s,
            right: self.ox + (r.x + r.w) * s,
            bottom: self.oy + (r.y + r.h) * s,
        }
    }

    /// One line of text, vertically centred in `r`, trimmed with an ellipsis if too long.
    pub fn text(&self, s: &str, font: Font, ink: Ink, r: Rect, align: Align) {
        let fmt = self.fonts.get(font);
        let wide: Vec<u16> = s.encode_utf16().collect();
        unsafe {
            let _ = fmt.SetTextAlignment(match align {
                Align::Left => DWRITE_TEXT_ALIGNMENT_LEADING,
                Align::Center => DWRITE_TEXT_ALIGNMENT_CENTER,
                Align::Right => DWRITE_TEXT_ALIGNMENT_TRAILING,
            });
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

    /// Width of `s` set on one line in `font`.
    pub fn measure(&self, s: &str, font: Font) -> f32 {
        let wide: Vec<u16> = s.encode_utf16().collect();
        let mut metrics = DWRITE_TEXT_METRICS::default();
        unsafe {
            let Ok(layout) = self.fonts.dwrite.CreateTextLayout(&wide, self.fonts.get(font), f32::MAX, f32::MAX) else {
                return 0.0;
            };
            // The format is shared, so it carries whatever alignment `text` last drew with;
            // centred or trailing in an f32::MAX-wide box, the width is lost to rounding.
            let _ = layout.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_LEADING);
            match layout.GetMetrics(&mut metrics) {
                Ok(()) => metrics.widthIncludingTrailingWhitespace / self.scale,
                Err(_) => 0.0,
            }
        }
    }

    pub fn fill(&self, r: Rect, radius: f32, ink: Ink) {
        let rr = D2D1_ROUNDED_RECT { rect: self.px(r), radiusX: radius * self.scale, radiusY: radius * self.scale };
        unsafe { self.rt.FillRoundedRectangle(&rr, self.brushes.get(ink)) };
    }

    /// A 1px line.
    pub fn line(&self, (x0, y0): (f32, f32), (x1, y1): (f32, f32), ink: Ink) {
        let s = self.scale;
        let p = |x: f32, y: f32| Vector2 { X: self.ox + x * s, Y: self.oy + y * s };
        unsafe { self.rt.DrawLine(p(x0, y0), p(x1, y1), self.brushes.get(ink), s, None) };
    }

    /// A tab: rounded by `radius` at the top, square at the bottom. `fill` paints it; `edge`
    /// outlines its top and sides but not its bottom, so it opens onto whatever is below.
    pub fn tab(&self, r: Rect, radius: f32, fill: Option<Ink>, edge: Option<Ink>) {
        let s = self.scale;
        let outline = |inset: f32, closed: bool| -> Option<ID2D1PathGeometry> {
            let px = self.px(r);
            let (left, top, right, bottom) = (px.left + inset, px.top + inset, px.right - inset, px.bottom);
            let rad = (radius * s - inset).max(0.0);
            let corner = |x: f32, y: f32| D2D1_ARC_SEGMENT {
                point: Vector2 { X: x, Y: y },
                size: D2D_SIZE_F { width: rad, height: rad },
                rotationAngle: 0.0,
                sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                arcSize: D2D1_ARC_SIZE_SMALL,
            };
            unsafe {
                let path = self.rt.GetFactory().ok()?.CreatePathGeometry().ok()?;
                let sink = path.Open().ok()?;
                let begin = if closed { D2D1_FIGURE_BEGIN_FILLED } else { D2D1_FIGURE_BEGIN_HOLLOW };
                sink.BeginFigure(Vector2 { X: left, Y: bottom }, begin);
                sink.AddLine(Vector2 { X: left, Y: top + rad });
                sink.AddArc(&corner(left + rad, top));
                sink.AddLine(Vector2 { X: right - rad, Y: top });
                sink.AddArc(&corner(right, top + rad));
                sink.AddLine(Vector2 { X: right, Y: bottom });
                sink.EndFigure(if closed { D2D1_FIGURE_END_CLOSED } else { D2D1_FIGURE_END_OPEN });
                sink.Close().ok()?;
                Some(path)
            }
        };
        unsafe {
            if let Some(ink) = fill
                && let Some(path) = outline(0.0, true)
            {
                self.rt.FillGeometry(&path, self.brushes.get(ink), None);
            }
            // A 1px stroke centred half a pixel inside the edge.
            if let Some(ink) = edge
                && let Some(path) = outline(0.5 * s, false)
            {
                self.rt.DrawGeometry(&path, self.brushes.get(ink), s, None);
            }
        }
    }

    /// Filled circle, or an outline of `stroke` logical px.
    pub fn circle(&self, cx: f32, cy: f32, radius: f32, ink: Ink, stroke: Option<f32>) {
        let s = self.scale;
        let e = D2D1_ELLIPSE {
            point: Vector2 { X: self.ox + cx * s, Y: self.oy + cy * s },
            radiusX: radius * s,
            radiusY: radius * s,
        };
        unsafe {
            match stroke {
                Some(w) => self.rt.DrawEllipse(&e, self.brushes.get(ink), w * s, None),
                None => self.rt.FillEllipse(&e, self.brushes.get(ink)),
            }
        }
    }

    /// The notch body in glass. `r` may extend above y = 0 (to keep the top corners
    /// off-screen); gradients span only its visible part. `shadow` (0..1) scales a soft drop
    /// shadow; it must stay 0 while collapsed, since shadow pixels would catch clicks.
    pub fn glass(&self, r: Rect, radius: f32, shadow: f32) {
        let g = &self.brushes.glass;
        let s = self.scale;
        let px = self.px(r);
        let top = px.top.max(self.oy);
        let (w, h) = (px.right - px.left, px.bottom - top);
        let rounded = |rect: D2D_RECT_F, radius: f32| D2D1_ROUNDED_RECT { rect, radiusX: radius, radiusY: radius };
        let body = rounded(px, radius * s);

        unsafe {
            // Concentric, increasingly faint layers approximate a blurred shadow.
            if shadow > 0.0 {
                const LAYERS: usize = 10;
                for i in (1..=LAYERS).rev() {
                    let grow = i as f32 * 1.6 * s;
                    let k = 1.0 - i as f32 / (LAYERS + 1) as f32;
                    g.shadow.SetOpacity(0.06 * k * k * shadow * g.opacity);
                    let rect = D2D_RECT_F {
                        left: px.left - grow,
                        top: px.top,
                        right: px.right + grow,
                        bottom: px.bottom + grow * 1.2,
                    };
                    self.rt.FillRoundedRectangle(&rounded(rect, radius * s + grow), &g.shadow);
                }
            }

            g.base.SetStartPoint(Vector2 { X: px.left, Y: top });
            g.base.SetEndPoint(Vector2 { X: px.left, Y: px.bottom });
            self.rt.FillRoundedRectangle(&body, &g.base);

            g.sheen.SetCenter(Vector2 { X: px.left + w * 0.28, Y: top });
            g.sheen.SetRadiusX(w * 0.55);
            g.sheen.SetRadiusY(h * 0.9);
            self.rt.FillRoundedRectangle(&body, &g.sheen);

            g.tint.SetCenter(Vector2 { X: px.left + w * 0.85, Y: px.bottom });
            g.tint.SetRadiusX(w * 0.5);
            g.tint.SetRadiusY(h * 1.2);
            self.rt.FillRoundedRectangle(&body, &g.tint);

            // A 1px stroke centred half a pixel inside the edge.
            let inset = 0.5 * s;
            let edge = D2D_RECT_F {
                left: px.left + inset,
                top: px.top + inset,
                right: px.right - inset,
                bottom: px.bottom - inset,
            };
            g.rim.SetStartPoint(Vector2 { X: px.left, Y: top });
            g.rim.SetEndPoint(Vector2 { X: px.left, Y: px.bottom });
            self.rt.DrawRoundedRectangle(&rounded(edge, radius * s - inset), &g.rim, s, None);
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
    use super::Rgb;
    use crate::settings::DEFAULT_COLOR;

    #[test]
    fn hex_round_trips() {
        assert_eq!(Rgb::from_hex(0x12AB3C), Rgb(0x12, 0xAB, 0x3C));
        assert_eq!(Rgb(0x12, 0xAB, 0x3C).hex(), 0x12AB3C);
    }

    #[test]
    fn default_colour_keeps_the_obsidian_look() {
        let c = Rgb::from_hex(DEFAULT_COLOR);
        assert_eq!(c.lighten(0.03), Rgb(18, 16, 25), "gradient top");
        assert_eq!(c.darken(0.4), Rgb(4, 4, 7), "gradient bottom");
        assert_eq!(c.glow(), Rgb(122, 92, 230), "violet glow");
    }

    #[test]
    fn glow_keeps_the_hue() {
        let Rgb(r, g, b) = Rgb(20, 60, 30).glow();
        assert!(g > r && g > b, "green stays green");
        let Rgb(r, g, b) = Rgb(80, 10, 20).glow();
        assert!(r > g && r > b, "red stays red");
        assert_eq!(Rgb(40, 40, 40).glow(), Rgb(230, 230, 236), "grey glows white");
    }
}
