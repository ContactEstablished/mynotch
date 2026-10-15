//! Turning pixels from elsewhere into `gfx::Image`s: WIC for encoded images (album art), and a
//! plain box filter for raw BGRA (clipboard bitmaps, icons) that needs no COM.

use windows::{
    Win32::{
        Graphics::Imaging::*,
        System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance},
    },
    core::GUID,
};

use crate::gfx::Image;

/// Size that fits `w` x `h` within `max_side`, keeping the aspect ratio (never upscales).
fn fit(w: u32, h: u32, max_side: u32) -> (u32, u32) {
    if w <= max_side && h <= max_side {
        return (w.max(1), h.max(1));
    }
    let k = max_side as f64 / w.max(h) as f64;
    (((w as f64 * k).round() as u32).max(1), ((h as f64 * k).round() as u32).max(1))
}

/// Decodes an encoded image (PNG, JPEG, BMP, ...) scaled to fit within `max_side`.
/// The calling thread must have COM initialized.
pub fn decode(bytes: &[u8], max_side: u32) -> Option<Image> {
    unsafe {
        let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()?;
        let stream = wic.CreateStream().ok()?;
        stream.InitializeFromMemory(bytes).ok()?;
        let decoder = wic.CreateDecoderFromStream(&stream, std::ptr::null::<GUID>(), WICDecodeMetadataCacheOnDemand).ok()?;
        let frame = decoder.GetFrame(0).ok()?;
        let (mut w, mut h) = (0, 0);
        frame.GetSize(&mut w, &mut h).ok()?;
        let (tw, th) = fit(w, h, max_side);
        let scaler = wic.CreateBitmapScaler().ok()?;
        scaler.Initialize(&frame, tw, th, WICBitmapInterpolationModeFant).ok()?;
        let converter = wic.CreateFormatConverter().ok()?;
        converter
            .Initialize(
                &scaler,
                &GUID_WICPixelFormat32bppPBGRA,
                WICBitmapDitherTypeNone,
                None::<&IWICPalette>,
                0.0,
                WICBitmapPaletteTypeCustom,
            )
            .ok()?;
        let mut pixels = vec![0u8; tw as usize * th as usize * 4];
        converter.CopyPixels(std::ptr::null(), tw * 4, &mut pixels).ok()?;
        Image::new(tw, th, pixels)
    }
}

/// Converts top-down BGRA rows (`stride` bytes apart; straight alpha when `straight`) into a
/// premultiplied `Image` that fits within `max_side`, averaging blocks of source pixels.
pub fn from_bgra(width: u32, height: u32, stride: usize, src: &[u8], straight: bool, max_side: u32) -> Option<Image> {
    let (w, h) = (width as usize, height as usize);
    if w == 0 || h == 0 || stride < w * 4 || src.len() < stride * (h - 1) + w * 4 {
        return None;
    }
    let (tw, th) = fit(width, height, max_side);
    let (tw, th) = (tw as usize, th as usize);
    let mut out = vec![0u8; tw * th * 4];
    for ty in 0..th {
        let (y0, y1) = (ty * h / th, ((ty + 1) * h / th).max(ty * h / th + 1));
        for tx in 0..tw {
            let (x0, x1) = (tx * w / tw, ((tx + 1) * w / tw).max(tx * w / tw + 1));
            let mut sum = [0u64; 4];
            for y in y0..y1 {
                for x in x0..x1 {
                    let p = &src[y * stride + x * 4..][..4];
                    let a = u64::from(p[3]);
                    for c in 0..3 {
                        let v = u64::from(p[c]);
                        sum[c] += if straight { v * a / 255 } else { v };
                    }
                    sum[3] += a;
                }
            }
            let n = ((y1 - y0) * (x1 - x0)) as u64;
            let o = &mut out[(ty * tw + tx) * 4..][..4];
            for c in 0..4 {
                o[c] = ((sum[c] + n / 2) / n) as u8;
            }
        }
    }
    Image::new(tw as u32, th as u32, out)
}

#[cfg(test)]
mod tests {
    use super::{fit, from_bgra};

    #[test]
    fn fits_within_max_side() {
        assert_eq!(fit(1280, 720, 320), (320, 180));
        assert_eq!(fit(100, 400, 200), (50, 200));
        assert_eq!(fit(64, 64, 320), (64, 64), "never upscales");
    }

    #[test]
    fn box_filter_averages_and_premultiplies() {
        // 2x2 straight-alpha pixels: white opaque, white half-transparent, two clear.
        #[rustfmt::skip]
        let src = [255, 255, 255, 255,   255, 255, 255, 128,
                   0, 0, 0, 0,           0, 0, 0, 0];
        let img = from_bgra(2, 2, 8, &src, true, 1).unwrap();
        assert_eq!((img.width, img.height), (1, 1));
        // Premultiplied: (255 + 128 + 0 + 0) / 4 = 96 for colour and alpha alike.
        assert_eq!(img.pixel(0, 0), [96, 96, 96, 96]);
    }

    #[test]
    fn rejects_short_buffers() {
        assert!(from_bgra(4, 4, 16, &[0; 20], false, 8).is_none());
    }
}
