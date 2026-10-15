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

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ink {
    Body,
    Text,
    Dim,
    Track,
    Hover,
    Accent,
}

/// Device-dependent brushes; rebuilt together with the render target.
pub struct Brushes {
    body: ID2D1SolidColorBrush,
    text: ID2D1SolidColorBrush,
    dim: ID2D1SolidColorBrush,
    track: ID2D1SolidColorBrush,
    hover: ID2D1SolidColorBrush,
    accent: ID2D1SolidColorBrush,
}

impl Brushes {
    pub unsafe fn new(rt: &ID2D1DCRenderTarget) -> Result<Self> {
        let solid = |r: f32, g: f32, b: f32| unsafe {
            rt.CreateSolidColorBrush(&D2D1_COLOR_F { r, g, b, a: 1.0 }, None)
        };
        Ok(Self {
            body: solid(0.0, 0.0, 0.0)?,
            text: solid(0.95, 0.95, 0.95)?,
            dim: solid(0.6, 0.6, 0.65)?,
            track: solid(0.2, 0.2, 0.24)?,
            hover: solid(0.28, 0.28, 0.33)?,
            accent: solid(0.35, 0.65, 1.0)?,
        })
    }

    fn get(&self, ink: Ink) -> &ID2D1SolidColorBrush {
        match ink {
            Ink::Body => &self.body,
            Ink::Text => &self.text,
            Ink::Dim => &self.dim,
            Ink::Track => &self.track,
            Ink::Hover => &self.hover,
            Ink::Accent => &self.accent,
        }
    }

    /// Opacity of every ink except the body, used to fade content in and out.
    pub fn set_content_opacity(&self, a: f32) {
        unsafe {
            for b in [&self.text, &self.dim, &self.track, &self.hover, &self.accent] {
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
}

/// Single-line text formats (no wrapping, ellipsis on overflow), sized for one DPI scale.
pub struct Fonts {
    small: IDWriteTextFormat,
    label: IDWriteTextFormat,
    title: IDWriteTextFormat,
    icon: IDWriteTextFormat,
    icon_medium: IDWriteTextFormat,
    icon_large: IDWriteTextFormat,
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

    /// Restricts drawing to `r` until `pop_clip`.
    pub fn push_clip(&self, r: Rect) {
        unsafe { self.rt.PushAxisAlignedClip(&self.px(r), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE) };
    }

    pub fn pop_clip(&self) {
        unsafe { self.rt.PopAxisAlignedClip() };
    }
}
