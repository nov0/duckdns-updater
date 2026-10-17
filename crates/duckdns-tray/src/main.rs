// No console window on Windows.
#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(any(windows, target_os = "macos"))]
mod app;
#[cfg(any(windows, target_os = "macos"))]
mod autostart;
#[cfg(any(windows, target_os = "macos"))]
mod config;
#[cfg(any(windows, target_os = "macos"))]
mod draw;
#[cfg(any(windows, target_os = "macos"))]
mod icon;

#[cfg(any(windows, target_os = "macos"))]
fn main() {
    app::run();
}

#[cfg(not(any(windows, target_os = "macos")))]
fn main() {
    eprintln!("duckdns-tray runs on Windows and macOS. On Linux/Docker use duckdns-daemon.");
    std::process::exit(1);
}
