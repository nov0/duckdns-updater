//! Windows: gives the .exe its icon in Explorer, the taskbar and the
//! "Apps" list. Draws the same duck as the tray (src/draw.rs) into an .ico
//! and embeds it as a resource. Nothing to do on other platforms.

#[path = "src/draw.rs"]
mod draw;

use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=src/draw.rs");
    println!("cargo:rerun-if-changed=build.rs");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let ico = PathBuf::from(env::var("OUT_DIR").unwrap()).join("app.ico");
    fs::write(&ico, make_ico(&[16, 20, 24, 32, 40, 48, 64, 256])).expect("cannot write app.ico");

    let mut res = winresource::WindowsResource::new();
    res.set_icon(ico.to_str().unwrap());
    res.set("FileDescription", "DuckDNS Updater");
    res.set("ProductName", "DuckDNS Updater");
    res.compile().expect("cannot embed Windows icon resource");
}

/// An .ico file holding one PNG image per size (supported since Vista).
fn make_ico(sizes: &[u32]) -> Vec<u8> {
    let images: Vec<Vec<u8>> = sizes.iter().map(|&s| png(&draw::with_margin(s), s)).collect();

    let mut ico = vec![0, 0, 1, 0]; // reserved, type 1 = icon
    ico.extend_from_slice(&(sizes.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * sizes.len() as u32;
    for (&size, image) in sizes.iter().zip(&images) {
        let dim = if size >= 256 { 0 } else { size as u8 }; // 0 means 256
        ico.extend_from_slice(&[dim, dim, 0, 0]); // w, h, no palette, reserved
        ico.extend_from_slice(&1u16.to_le_bytes()); // colour planes
        ico.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
        ico.extend_from_slice(&(image.len() as u32).to_le_bytes());
        ico.extend_from_slice(&offset.to_le_bytes());
        offset += image.len() as u32;
    }
    for image in images {
        ico.extend_from_slice(&image);
    }
    ico
}

fn png(rgba: &[u8], size: u32) -> Vec<u8> {
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, size, size);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_compression(png::Compression::High);
    let mut w = enc.write_header().expect("png header");
    w.write_image_data(rgba).expect("png data");
    w.finish().expect("png finish");
    out
}
