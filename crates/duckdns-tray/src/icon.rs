//! Tray icons are drawn in code (src/draw.rs), so the .exe needs no extra files.

use crate::draw;
use tray_icon::Icon;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Ok,
    Working,
    Error,
}

const SIZE: u32 = 32;

/// The DuckDNS-style rubber duck with a status dot in the bottom-right corner.
pub fn make(state: State) -> Icon {
    let dot = match state {
        State::Ok => (46, 160, 67),
        State::Working => (210, 153, 34),
        State::Error => (207, 34, 46),
    };
    Icon::from_rgba(draw::rgba(SIZE, Some(dot)), SIZE, SIZE).expect("valid icon buffer")
}
