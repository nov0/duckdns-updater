//! Config file, log file and instance lock.
//! Windows: %APPDATA%\DuckDNS Updater
//! macOS:   ~/Library/Application Support/DuckDNS Updater

use duckdns_core::{parse_domains, Settings};
use serde::Deserialize;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

const TEMPLATE: &str = r#"# DuckDNS Updater settings.
# Changes are picked up automatically on the next check,
# or right away with "Update now" in the tray menu.

# Your subdomains WITHOUT ".duckdns.org", comma separated.
domains = "myhome"

# Token shown at the top of https://www.duckdns.org after you log in.
token = "your-token-here"

# How often to check your public IP, in minutes.
interval_minutes = 5

# Also publish an IPv6 (AAAA) record.
ipv6 = false
"#;

#[derive(Debug, Deserialize)]
pub struct FileConfig {
    #[serde(default)]
    pub domains: String,
    #[serde(default)]
    pub token: String,
    #[serde(default = "default_interval")]
    pub interval_minutes: u64,
    #[serde(default)]
    pub ipv6: bool,
}

fn default_interval() -> u64 {
    5
}

impl FileConfig {
    pub fn settings(&self) -> Settings {
        Settings {
            domains: parse_domains(&self.domains),
            token: self.token.clone(),
            ipv6: self.ipv6,
        }
    }

    pub fn interval(&self) -> Duration {
        Duration::from_secs(self.interval_minutes.clamp(1, 24 * 60) * 60)
    }
}

fn base_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    return std::env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(target_os = "macos")]
    return std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"));
}

pub fn app_dir() -> PathBuf {
    let dir = base_dir().unwrap_or_else(std::env::temp_dir).join("DuckDNS Updater");
    let _ = fs::create_dir_all(&dir);
    dir
}

pub fn config_path() -> PathBuf {
    app_dir().join("config.toml")
}

pub fn log_path() -> PathBuf {
    app_dir().join("log.txt")
}

/// Writes the commented template if no config exists yet.
/// Returns true on first run.
pub fn ensure_exists() -> bool {
    let path = config_path();
    if path.exists() {
        return false;
    }
    let _ = fs::write(&path, TEMPLATE);
    true
}

pub fn load() -> Result<FileConfig, String> {
    let text = fs::read_to_string(config_path()).map_err(|e| format!("cannot read config: {e}"))?;
    toml::from_str(&text).map_err(|e| format!("config.toml is invalid: {}", e.message()))
}

/// Holds an exclusive lock for the lifetime of the process so a second
/// copy (e.g. autostart + manual launch) exits instead of adding a 2nd icon.
pub fn acquire_instance_lock() -> Option<File> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(app_dir().join("instance.lock"))
        .ok()?;
    file.try_lock().ok()?;
    Some(file)
}

/// Appends a timestamped line; rotates once the file passes 1 MB.
pub fn log(msg: &str) {
    let path = log_path();
    if fs::metadata(&path).map_or(false, |m| m.len() > 1_000_000) {
        let _ = fs::rename(&path, app_dir().join("log.old.txt"));
    }
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&path) {
        let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
        let _ = writeln!(f, "{now}  {msg}");
    }
}

/// Opens a text file in Notepad (Windows) or the default text editor (macOS).
pub fn open_in_editor(path: &Path) {
    #[cfg(windows)]
    let result = Command::new("notepad.exe").arg(path).spawn();
    #[cfg(target_os = "macos")]
    let result = Command::new("open").arg("-t").arg(path).spawn();

    if let Err(e) = result {
        log(&format!("Could not open {}: {e}", path.display()));
    }
}
