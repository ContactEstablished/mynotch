//! Drawing primitives shared by the shell and widgets. Widgets lay out in logical (96-DPI)
//! units relative to their own origin; `Canvas` maps that onto physical pixels.

use windows::{
    Win32::Graphics::{
        Direct2D::{Common::*, *},
        DirectWrite::*,
    },
    core::*,
};
use windows_numerics::Vector2;

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
}

const fn rgba(r: u8, g: u8, b: u8, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r: r as f32 / 255.0, g: g as f32 / 255.0, b: b as f32 / 255.0, a }
}

/// The obsidian body material (see `Canvas::glass`). Gradient geometry follows the
/// animating notch, so it is set on every draw.
struct Glass {
    /// Near-black, slightly see-through; lifts a touch towards the top.
    base: ID2D1LinearGradientBrush,
    /// Soft reflection of light from the upper left.
    sheen: ID2D1RadialGradientBrush,
    /// Faint violet glow in the lower right, like obsidian's sheen.
    tint: ID2D1RadialGradientBrush,
    /// Light catching the edge, strongest along the bottom.
    rim: ID2D1LinearGradientBrush,
    shadow: ID2D1SolidColorBrush,
}

impl Glass {
    unsafe fn new(rt: &ID2D1DCRenderTarget) -> Result<Self> {
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
            Ok(Self {
                base: linear(&[(0.0, rgba(17, 17, 24, 0.96)), (1.0, rgba(4, 4, 7, 0.97))])?,
                sheen: radial(&[(0.0, rgba(255, 255, 255, 0.10)), (1.0, rgba(255, 255, 255, 0.0))])?,
                tint: radial(&[(0.0, rgba(124, 92, 230, 0.14)), (1.0, rgba(124, 92, 230, 0.0))])?,
                rim: linear(&[(0.0, rgba(255, 255, 255, 0.04)), (1.0, rgba(255, 255, 255, 0.28))])?,
                shadow: rt.CreateSolidColorBrush(&rgba(0, 0, 0, 1.0), None)?,
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
    glass: Glass,
}

impl Brushes {
    pub unsafe fn new(rt: &ID2D1DCRenderTarget) -> Result<Self> {
        let solid = |c: D2D1_COLOR_F| unsafe { rt.CreateSolidColorBrush(&c, None) };
        Ok(Self {
            text: solid(rgba(242, 242, 247, 1.0))?,
            dim: solid(rgba(158, 160, 176, 1.0))?,
            track: solid(rgba(255, 255, 255, 0.09))?,
            hover: solid(rgba(255, 255, 255, 0.16))?,
            accent: solid(rgba(122, 176, 255, 1.0))?,
            calm: solid(rgba(102, 214, 184, 1.0))?,
            glass: unsafe { Glass::new(rt)? },
        })
    }

    fn get(&self, ink: Ink) -> &ID2D1SolidColorBrush {
        match ink {
            Ink::Text => &self.text,
            Ink::Dim => &self.dim,
            Ink::Track => &self.track,
            Ink::Hover => &self.hover,
            Ink::Accent => &self.accent,
            Ink::Calm => &self.calm,
        }
    }

    /// Opacity of every content ink (not the body), used to fade content in and out.
    pub fn set_content_opacity(&self, a: f32) {
        unsafe {
            for b in [&self.text, &self.dim, &self.track, &self.hover, &self.accent, &self.calm] {
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
}

/// Single-line text formats (no wrapping, ellipsis on overflow), sized for one DPI scale.
pub struct Fonts {
    small: IDWriteTextFormat,
    label: IDWriteTextFormat,
    title: IDWriteTextFormat,
    icon: IDWriteTextFormat,
    icon_medium: IDWriteTextFormat,
    icon_large: IDWriteTextFormat,
    display: IDWriteTextFormat,
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
                small: make(ui, 12.0, DWRITE_FONT_WEIGHT_NORMAL)?,
                label: make(ui, 13.0, DWRITE_FONT_WEIGHT_SEMI_BOLD)?,
                title: make(ui, 15.0, DWRITE_FONT_WEIGHT_SEMI_BOLD)?,
                icon: make(icons, 16.0, DWRITE_FONT_WEIGHT_NORMAL)?,
                icon_medium: make(icons, 20.0, DWRITE_FONT_WEIGHT_NORMAL)?,
                icon_large: make(icons, 24.0, DWRITE_FONT_WEIGHT_NORMAL)?,
                display: make(ui, 32.0, DWRITE_FONT_WEIGHT_LIGHT)?,
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
        scale: f32,
        origin: (f32, f32),
        cursor: Option<(f32, f32)>,
    ) -> Self {
        Self { rt, brushes, fonts, scale, ox: origin.0, oy: origin.1, cursor }
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

    pub fn fill(&self, r: Rect, radius: f32, ink: Ink) {
        let rr = D2D1_ROUNDED_RECT { rect: self.px(r), radiusX: radius * self.scale, radiusY: radius * self.scale };
        unsafe { self.rt.FillRoundedRectangle(&rr, self.brushes.get(ink)) };
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

    /// The notch body in obsidian glass. `r` may extend above y = 0 (to keep the top corners
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
                    g.shadow.SetOpacity(0.06 * k * k * shadow);
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

    /// A circular arc with round caps, as a fraction of a full turn starting at 12 o'clock
    /// and running clockwise (`fraction` 0..=1).
    pub fn arc(&self, cx: f32, cy: f32, radius: f32, fraction: f32, width: f32, ink: Ink) {
        let fraction = fraction.clamp(0.0, 1.0);
        if fraction <= 0.0 {
            return;
        }
        let s = self.scale;
        let (cx, cy, r) = (self.ox + cx * s, self.oy + cy * s, radius * s);
        let brush = self.brushes.get(ink);
        unsafe {
            if fraction >= 0.9999 {
                let e = D2D1_ELLIPSE { point: Vector2 { X: cx, Y: cy }, radiusX: r, radiusY: r };
                self.rt.DrawEllipse(&e, brush, width * s, None);
                return;
            }
            let Ok(factory) = self.rt.GetFactory() else { return };
            let Ok(path) = factory.CreatePathGeometry() else { return };
            let Ok(sink) = path.Open() else { return };
            let angle = fraction * std::f32::consts::TAU;
            sink.BeginFigure(Vector2 { X: cx, Y: cy - r }, D2D1_FIGURE_BEGIN_HOLLOW);
            sink.AddArc(&D2D1_ARC_SEGMENT {
                point: Vector2 { X: cx + r * angle.sin(), Y: cy - r * angle.cos() },
                size: D2D_SIZE_F { width: r, height: r },
                rotationAngle: 0.0,
                sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                arcSize: if fraction > 0.5 { D2D1_ARC_SIZE_LARGE } else { D2D1_ARC_SIZE_SMALL },
            });
            sink.EndFigure(D2D1_FIGURE_END_OPEN);
            if sink.Close().is_err() {
                return;
            }
            let round = D2D1_STROKE_STYLE_PROPERTIES {
                startCap: D2D1_CAP_STYLE_ROUND,
                endCap: D2D1_CAP_STYLE_ROUND,
                dashCap: D2D1_CAP_STYLE_ROUND,
                lineJoin: D2D1_LINE_JOIN_ROUND,
                miterLimit: 10.0,
                dashStyle: D2D1_DASH_STYLE_SOLID,
                dashOffset: 0.0,
            };
            let style = factory.CreateStrokeStyle(&round, None).ok();
            self.rt.DrawGeometry(&path, brush, width * s, style.as_ref());
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
