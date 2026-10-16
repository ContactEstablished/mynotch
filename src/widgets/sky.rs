//! Small drawn icons for the sky: the sun, the moon in its real phase, and the weather.

use std::f32::consts::TAU;

use crate::gfx::{Canvas, Ink, Path, Rect, Shape};
use crate::theme::color;
use crate::weather::Sky;

const SUN: Ink = Ink::Rgb(color::SUN);
const MOON: Ink = Ink::Rgb(color::MOON);

/// The sun by day, else the moon at `age` (see `weather::moon_age`), within radius `r`.
/// `south`: as seen from the southern hemisphere, where the moon's lit side is mirrored.
pub fn body(cv: &Canvas, (cx, cy): (f32, f32), r: f32, day: bool, age: f32, south: bool) {
    if day {
        sun(cv, cx, cy, r);
    } else {
        moon(cv, cx, cy, r, age, south);
    }
}

fn sun(cv: &Canvas, cx: f32, cy: f32, r: f32) {
    let core = r * 0.46;
    cv.halo(cx, cy, core, SUN, 0.45);
    cv.circle(cx, cy, core, SUN, None);
    for i in 0..8 {
        let (s, c) = (i as f32 * TAU / 8.0).sin_cos();
        let width = (r * 0.15).max(1.0);
        cv.stroke_line((cx + c * r * 0.72, cy + s * r * 0.72), (cx + c * r, cy + s * r), SUN, width, 1.0);
    }
}

/// The lit part runs from the bright limb (right while waxing, in the north) across to the
/// terminator: a half-ellipse that bulges towards the lit side for a crescent and away from
/// it once more than half is lit.
fn moon(cv: &Canvas, cx: f32, cy: f32, r: f32, age: f32, south: bool) {
    let lit = (1.0 - (age * TAU).cos()) / 2.0;
    cv.circle(cx, cy, r, Ink::Track, None);
    cv.circle_a(cx, cy, r - 0.5, Ink::Edge, Some(1.0), 0.6);
    if lit < 0.02 {
        return;
    }
    cv.halo(cx, cy, r, MOON, 0.15 + 0.25 * lit);
    if lit > 0.98 {
        cv.circle(cx, cy, r, MOON, None);
        return;
    }
    let right = (age < 0.5) != south;
    let half = (r * (1.0 - 2.0 * lit).abs()).max(0.01);
    // Top to bottom through the lit side; clockwise on screen is through the right.
    let limb_clockwise = right;
    let terminator_clockwise = if lit < 0.5 { !right } else { right };
    let path = Path::new()
        .move_to(cx, cy - r)
        .arc_to(cx, cy + r, (r, r), false, limb_clockwise)
        .arc_to(cx, cy - r, (half, r), false, terminator_clockwise)
        .close();
    if let Some(shape) = cv.path_shape(&path) {
        cv.fill_shape(&shape, MOON, 1.0);
    }
}

/// The weather as a small icon `size` px across, centred on (`cx`, `cy`): the sun or moon
/// when clear (peeping out when partly cloudy), a cloud (with rain, snow or lightning), fog,
/// or wind when it's windy and otherwise dry.
#[allow(clippy::too_many_arguments)]
pub fn weather(cv: &Canvas, (cx, cy): (f32, f32), size: f32, sky: Sky, windy: bool, day: bool, age: f32, south: bool) {
    let s = size;
    let dry = matches!(sky, Sky::Clear | Sky::PartlyCloudy | Sky::Cloudy);
    if windy && dry {
        wind(cv, cx, cy, s);
        return;
    }
    match sky {
        Sky::Clear => body(cv, (cx, cy), s * 0.42, day, age, south),
        Sky::PartlyCloudy => {
            let Some(cover) = cloud_shape(cv, cx - s * 0.06, cy + s * 0.1, s * 0.92) else { return };
            // The sun or moon behind, showing only outside the cloud.
            if let Some(mask) = cv.outside(&cover) {
                cv.masked(&mask, || body(cv, (cx + s * 0.18, cy - s * 0.16), s * 0.27, day, age, south));
            }
            draw_cloud(cv, &cover);
        }
        Sky::Cloudy => {
            if let Some(c) = cloud_shape(cv, cx, cy + s * 0.04, s) {
                draw_cloud(cv, &c);
            }
        }
        Sky::Fog => {
            let w = (s * 0.09).max(1.0);
            for (y, x0, x1) in [(-0.2, -0.38, 0.3), (0.0, -0.3, 0.38), (0.2, -0.38, 0.22)] {
                cv.stroke_line((cx + x0 * s, cy + y * s), (cx + x1 * s, cy + y * s), Ink::Dim, w, 1.0);
            }
        }
        Sky::Rain | Sky::Snow | Sky::Storm => {
            // Lightning needs more room below, so its cloud sits higher and smaller.
            let (lift, scale) = if sky == Sky::Storm { (0.18, 0.84) } else { (0.12, 0.92) };
            if let Some(c) = cloud_shape(cv, cx, cy - s * lift, s * scale) {
                draw_cloud(cv, &c);
            }
            let below = cy + s * 0.16;
            match sky {
                Sky::Rain => {
                    for x in [-0.16, 0.0, 0.16] {
                        let (x, w) = (cx + x * s, (s * 0.08).max(1.0));
                        cv.stroke_line((x + s * 0.04, below), (x - s * 0.03, below + s * 0.2), Ink::Accent, w, 1.0);
                    }
                }
                Sky::Snow => {
                    for (x, y) in [(-0.17, 0.06), (0.0, 0.2), (0.17, 0.06)] {
                        cv.circle(cx + x * s, below + y * s, (s * 0.055).max(0.9), Ink::Text, None);
                    }
                }
                _ => {
                    let at = |x: f32, y: f32| (cx + x * s, below + y * s);
                    let points = [(0.1, -0.14), (-0.14, 0.14), (0.0, 0.14), (-0.1, 0.38), (0.18, 0.04), (0.04, 0.04)];
                    let mut path = Path::new();
                    for (i, &(x, y)) in points.iter().enumerate() {
                        let (px, py) = at(x, y);
                        path = if i == 0 { path.move_to(px, py) } else { path.line_to(px, py) };
                    }
                    if let Some(bolt) = cv.path_shape(&path.close()) {
                        cv.fill_shape(&bolt, SUN, 1.0);
                    }
                }
            }
        }
    }
}

/// A cloud `s` px across: three puffs on a flat base.
fn cloud_shape(cv: &Canvas, cx: f32, cy: f32, s: f32) -> Option<Shape> {
    let puffs = [(cx - s * 0.22, cy + s * 0.04, s * 0.17), (cx + s * 0.02, cy - s * 0.1, s * 0.24), (cx + s * 0.23, cy + s * 0.06, s * 0.15)];
    cv.blob(&puffs, &[Rect::new(cx - s * 0.22, cy + s * 0.04, s * 0.45, s * 0.17)])
}

fn draw_cloud(cv: &Canvas, cloud: &Shape) {
    cv.fill_shape(cloud, Ink::Hover, 1.0);
    cv.stroke_shape(cloud, Ink::Text, 1.2, 0.9);
}

/// Three gusts, two of them curling at the end.
fn wind(cv: &Canvas, cx: f32, cy: f32, s: f32) {
    let (w, curl) = ((s * 0.08).max(1.0), s * 0.1);
    let gust = |y: f32, x0: f32, x1: f32, up: Option<bool>| {
        let (x0, x1, y) = (cx + x0 * s, cx + x1 * s, cy + y * s);
        let mut path = Path::new().move_to(x0, y).line_to(x1, y);
        if let Some(up) = up {
            let end = if up { y - curl * 2.0 } else { y + curl * 2.0 };
            // Round the end back on itself: up is anticlockwise on screen, down clockwise.
            path = path.arc_to(x1, end, (curl, curl), false, !up).arc_to(x1 - curl, end + if up { curl } else { -curl }, (curl, curl), false, !up);
        }
        if let Some(line) = cv.path_shape(&path) {
            cv.stroke_shape(&line, Ink::Text, w, 0.9);
        }
    };
    gust(-0.12, -0.42, 0.16, Some(true));
    gust(0.06, -0.36, 0.28, Some(false));
    gust(0.26, -0.42, 0.04, None);
}
