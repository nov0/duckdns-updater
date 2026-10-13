//! Tray icons are drawn in code, so the .exe needs no extra files.

use tray_icon::Icon;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Ok,
    Working,
    Error,
}

/// A filled, anti-aliased circle with a slightly darker rim.
pub fn make(state: State) -> Icon {
    let (r, g, b) = match state {
        State::Ok => (46, 160, 67),
        State::Working => (210, 153, 34),
        State::Error => (207, 34, 46),
    };
    const SIZE: u32 = 32;
    let c = (SIZE as f32 - 1.0) / 2.0;
    let radius = c - 1.0;
    let mut rgba = Vec::with_capacity((SIZE * SIZE * 4) as usize);

    for y in 0..SIZE {
        for x in 0..SIZE {
            let d = ((x as f32 - c).powi(2) + (y as f32 - c).powi(2)).sqrt();
            let alpha = (radius + 0.5 - d).clamp(0.0, 1.0); // soft edge
            let shade = if d > radius - 3.0 { 0.75 } else { 1.0 }; // rim
            rgba.extend_from_slice(&[
                (r as f32 * shade) as u8,
                (g as f32 * shade) as u8,
                (b as f32 * shade) as u8,
                (alpha * 255.0) as u8,
            ]);
        }
    }
    Icon::from_rgba(rgba, SIZE, SIZE).expect("valid icon buffer")
}
