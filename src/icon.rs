//! The tray icon, drawn at runtime so it is crisp at any DPI and matches the taskbar theme:
//! a screen outline with the notch, in the accent gradient, hanging from its top edge.

use windows::{
    Win32::{
        Foundation::RECT,
        Graphics::{
            Direct2D::{Common::*, *},
            Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM,
            Gdi::*,
        },
        UI::WindowsAndMessaging::{CreateIconIndirect, HICON, ICONINFO},
    },
    core::*,
};
use windows_numerics::Vector2;

/// Renders the icon at `size` x `size` pixels, for a light or dark taskbar.
pub fn render(d2d: &ID2D1Factory, size: i32, light_taskbar: bool) -> Result<HICON> {
    unsafe {
        let screen = GetDC(None);
        let dc = CreateCompatibleDC(Some(screen));
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size,
                biHeight: -size, // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let color = CreateDIBSection(Some(screen), &bmi, DIB_RGB_COLORS, &mut bits, None, 0);
        ReleaseDC(None, screen);
        let color = match color {
            Ok(c) => c,
            Err(e) => {
                let _ = DeleteDC(dc);
                return Err(e);
            }
        };
        let old = SelectObject(dc, color.into());
        let drawn = draw(d2d, dc, size, light_taskbar);
        SelectObject(dc, old);
        let _ = DeleteDC(dc);

        let icon = drawn.and_then(|()| {
            // D2D wrote premultiplied alpha; icon bitmaps use straight alpha.
            let _ = GdiFlush();
            let pixels = std::slice::from_raw_parts_mut(bits.cast::<[u8; 4]>(), (size * size) as usize);
            for p in pixels {
                let a = u32::from(p[3]);
                if a > 0 && a < 255 {
                    for c in &mut p[..3] {
                        *c = ((u32::from(*c) * 255 + a / 2) / a).min(255) as u8;
                    }
                }
            }
            // The mask is ignored for 32bpp colour bitmaps with alpha, but must exist.
            let mask = CreateBitmap(size, size, 1, 1, None);
            let icon = CreateIconIndirect(&ICONINFO {
                fIcon: true.into(),
                xHotspot: 0,
                yHotspot: 0,
                hbmMask: mask,
                hbmColor: color,
            });
            let _ = DeleteObject(mask.into());
            icon
        });
        let _ = DeleteObject(color.into());
        icon
    }
}

unsafe fn draw(d2d: &ID2D1Factory, dc: HDC, size: i32, light_taskbar: bool) -> Result<()> {
    unsafe {
        let props = D2D1_RENDER_TARGET_PROPERTIES {
            r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            usage: D2D1_RENDER_TARGET_USAGE_NONE,
            minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
        };
        let rt = d2d.CreateDCRenderTarget(&props)?;
        rt.BindDC(dc, &RECT { left: 0, top: 0, right: size, bottom: size })?;
        rt.BeginDraw();
        rt.Clear(Some(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }));

        let s = size as f32;
        let u = s / 16.0;
        // Stroke on whole pixels, with edges on pixel centres, so the outline stays sharp.
        let stroke = u.round().max(1.0);
        let half = stroke / 2.0;
        let line = if light_taskbar {
            D2D1_COLOR_F { r: 0.12, g: 0.12, b: 0.15, a: 1.0 }
        } else {
            D2D1_COLOR_F { r: 0.96, g: 0.96, b: 0.98, a: 1.0 }
        };
        let outline = rt.CreateSolidColorBrush(&line, None)?;
        let (left, top, right, bottom) = ((1.0 * u).round() + half, (2.0 * u).round() + half, s - (1.0 * u).round() - half, s - (2.0 * u).round() - half);
        let screen = D2D1_ROUNDED_RECT {
            rect: D2D_RECT_F { left, top, right, bottom },
            radiusX: 2.5 * u,
            radiusY: 2.5 * u,
        };
        rt.DrawRoundedRectangle(&screen, &outline, stroke, None);

        // The notch: hangs from the top edge, covering it, with rounded lower corners.
        let stops = [
            D2D1_GRADIENT_STOP { position: 0.0, color: D2D1_COLOR_F { r: 0.48, g: 0.69, b: 1.0, a: 1.0 } },
            D2D1_GRADIENT_STOP { position: 1.0, color: D2D1_COLOR_F { r: 0.62, g: 0.47, b: 1.0, a: 1.0 } },
        ];
        let stops = rt.CreateGradientStopCollection(&stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP)?;
        let (nw, nh, nr) = (7.0 * u, 4.0 * u, 2.0 * u);
        let (nl, nt) = ((s - nw) / 2.0, top - half);
        let gradient = rt.CreateLinearGradientBrush(
            &D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                startPoint: Vector2 { X: nl, Y: nt },
                endPoint: Vector2 { X: nl + nw, Y: nt + nh },
            },
            None,
            &stops,
        )?;
        rt.PushAxisAlignedClip(&D2D_RECT_F { left: 0.0, top: nt, right: s, bottom: s }, D2D1_ANTIALIAS_MODE_ALIASED);
        let notch = D2D1_ROUNDED_RECT {
            rect: D2D_RECT_F { left: nl, top: nt - nr, right: nl + nw, bottom: nt + nh },
            radiusX: nr,
            radiusY: nr,
        };
        rt.FillRoundedRectangle(&notch, &gradient);
        rt.PopAxisAlignedClip();

        rt.EndDraw(None, None)
    }
}
