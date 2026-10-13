//! Tray icons are drawn in code, so the .exe needs no extra files.
//! The same drawing, without the status dot, is the macOS app icon
//! (see examples/app_icon.rs and scripts/bundle-macos.sh).

use tray_icon::Icon;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Ok,
    Working,
    Error,
}

const SIZE: u32 = 32;
/// Each pixel averages SS×SS samples, which smooths (anti-aliases) the edges.
const SS: u32 = 4;
/// Outline width, in units of the icon size.
const STROKE: f32 = 0.065;

const BLACK: (u8, u8, u8) = (0, 0, 0);
const YELLOW: (u8, u8, u8) = (255, 240, 80);
const RED: (u8, u8, u8) = (232, 60, 40);

/// The DuckDNS-style rubber duck with a status dot in the bottom-right corner.
pub fn make(state: State) -> Icon {
    let dot = match state {
        State::Ok => (46, 160, 67),
        State::Working => (210, 153, 34),
        State::Error => (207, 34, 46),
    };
    Icon::from_rgba(rgba(SIZE, Some(dot)), SIZE, SIZE).expect("valid icon buffer")
}

/// The duck as a `size`×`size` RGBA buffer, with an optional status dot.
pub fn rgba(size: u32, dot: Option<(u8, u8, u8)>) -> Vec<u8> {
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);

    for py in 0..size {
        for px in 0..size {
            let (mut r, mut g, mut b, mut hits) = (0u32, 0u32, 0u32, 0u32);
            for sy in 0..SS {
                for sx in 0..SS {
                    let x = (px as f32 + (sx as f32 + 0.5) / SS as f32) / size as f32;
                    let y = (py as f32 + (sy as f32 + 0.5) / SS as f32) / size as f32;
                    if let Some(c) = colour_at(x, y, dot) {
                        r += c.0 as u32;
                        g += c.1 as u32;
                        b += c.2 as u32;
                        hits += 1;
                    }
                }
            }
            let n = hits.max(1);
            rgba.extend_from_slice(&[
                (r / n) as u8,
                (g / n) as u8,
                (b / n) as u8,
                (hits * 255 / (SS * SS)) as u8,
            ]);
        }
    }
    rgba
}

/// Colour at (x, y) in 0..1 coordinates (y points down), or None if transparent.
/// Shapes are signed distances: negative inside, positive outside; a band of
/// width STROKE just outside each shape is its black outline.
/// Checked front to back: dot, eye, duck, beak.
fn colour_at(x: f32, y: f32, dot: Option<(u8, u8, u8)>) -> Option<(u8, u8, u8)> {
    let (dot_dist, dot_fill) = match dot {
        Some(c) => (circle(x, y, 0.80, 0.80, 0.15), c),
        None => (f32::INFINITY, BLACK), // no dot: never hit
    };
    let layers = [
        (dot_dist, dot_fill),
        (circle(x, y, 0.65, 0.22, 0.045), BLACK),
        (duck(x, y), YELLOW),
        (ellipse(x, y, 0.80, 0.25, 0.10, 0.075, 0.0), RED),
    ];
    layers.into_iter().find_map(|(d, fill)| {
        if d < 0.0 {
            Some(fill)
        } else if d < STROKE && fill != BLACK {
            Some(BLACK)
        } else {
            None
        }
    })
}

/// Body, raised tail and head, blended into one outline.
fn duck(x: f32, y: f32) -> f32 {
    let body = ellipse(x, y, 0.46, 0.64, 0.34, 0.20, 0.0);
    let tail = ellipse(x, y, 0.23, 0.49, 0.085, 0.22, -0.3);
    let head = circle(x, y, 0.60, 0.26, 0.17);
    smooth_min(smooth_min(body, tail, 0.08), head, 0.06)
}

fn circle(x: f32, y: f32, cx: f32, cy: f32, r: f32) -> f32 {
    ((x - cx).powi(2) + (y - cy).powi(2)).sqrt() - r
}

/// Approximate distance to an ellipse rotated by `angle` radians.
fn ellipse(x: f32, y: f32, cx: f32, cy: f32, rx: f32, ry: f32, angle: f32) -> f32 {
    let (s, c) = angle.sin_cos();
    let (dx, dy) = (x - cx, y - cy);
    let (u, v) = ((dx * c + dy * s) / rx, (-dx * s + dy * c) / ry);
    ((u * u + v * v).sqrt() - 1.0) * rx.min(ry)
}

/// Union of two shapes with a rounded fillet of size `k` where they meet.
fn smooth_min(a: f32, b: f32, k: f32) -> f32 {
    let h = (k - (a - b).abs()).max(0.0) / k;
    a.min(b) - h * h * k * 0.25
}
