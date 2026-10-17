//! Writes the duck as a macOS .iconset folder (PNGs at every size macOS
//! wants), for `iconutil -c icns`. Used by scripts/bundle-macos.sh:
//!
//!     cargo run --example app_icon -- target/AppIcon.iconset

#[allow(dead_code)] // unused in the Linux stub
#[path = "../src/draw.rs"]
mod draw;

#[cfg(any(windows, target_os = "macos"))]
fn main() {
    let dir = std::env::args().nth(1).expect("usage: app_icon <output.iconset>");
    std::fs::create_dir_all(&dir).expect("cannot create output folder");
    for base in [16, 32, 128, 256, 512] {
        for (scale, suffix) in [(1, ""), (2, "@2x")] {
            let size = base * scale;
            let png = encode_png(&draw::with_margin(size), size);
            let path = format!("{dir}/icon_{base}x{base}{suffix}.png");
            std::fs::write(&path, png).expect("cannot write icon");
        }
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
fn main() {
    eprintln!("app_icon is only needed on macOS.");
}

/// Minimal PNG writer: 8-bit RGBA, uncompressed ("stored") deflate blocks.
/// Files are larger than with real compression, which iconutil doesn't mind.
#[cfg(any(windows, target_os = "macos"))]
fn encode_png(rgba: &[u8], size: u32) -> Vec<u8> {
    // Each row is prefixed with filter type 0 (none).
    let mut raw = Vec::with_capacity(rgba.len() + size as usize);
    for row in rgba.chunks(size as usize * 4) {
        raw.push(0);
        raw.extend_from_slice(row);
    }

    let mut zlib = vec![0x78, 0x01];
    let mut blocks = raw.chunks(65_535).peekable();
    while let Some(block) = blocks.next() {
        let len = block.len() as u16;
        zlib.push(u8::from(blocks.peek().is_none())); // BFINAL on the last block
        zlib.extend_from_slice(&len.to_le_bytes());
        zlib.extend_from_slice(&(!len).to_le_bytes());
        zlib.extend_from_slice(block);
    }
    zlib.extend_from_slice(&adler32(&raw).to_be_bytes());

    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&size.to_be_bytes());
    ihdr.extend_from_slice(&size.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // 8-bit, RGBA, defaults

    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    for (kind, data) in [(b"IHDR", &ihdr), (b"IDAT", &zlib), (b"IEND", &Vec::new())] {
        png.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let start = png.len();
        png.extend_from_slice(kind);
        png.extend_from_slice(data);
        let crc = crc32(&png[start..]);
        png.extend_from_slice(&crc.to_be_bytes());
    }
    png
}

#[cfg(any(windows, target_os = "macos"))]
fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

#[cfg(any(windows, target_os = "macos"))]
fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in data {
        a = (a + byte as u32) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
}
