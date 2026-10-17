//! Config file, log file and instance lock.
//! Windows: %APPDATA%\DuckDNS Updater
//! macOS:   ~/Library/Application Support/DuckDNS Updater

use duckdns_core::{parse_domains, Settings, MIN_INTERVAL};
use serde::Deserialize;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

const TEMPLATE: &str = r#"# DuckDNS Updater settings.
# Changes are picked up automatically on the next check,
# or right away with "Update now" in the tray menu.

# Your subdomains, comma separated ("myhome" or "myhome.duckdns.org").
domains = "myhome"

# Token shown at the top of https://www.duckdns.org after you log in.
token = "your-token-here"

# How often to check your public IP, in minutes.
interval_minutes = 5

# Also publish an IPv6 (AAAA) record.
ipv6 = false
"#;

// No Debug derive: it would print the token.
#[derive(Deserialize)]
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
        Duration::from_secs(self.interval_minutes.min(24 * 60) * 60).max(MIN_INTERVAL)
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

/// config.toml holds the token, so the folder and file are owner-only.
/// Also fixes files created by older versions with default permissions.
/// (Windows: %APPDATA% is already private to the user and is inherited.)
#[cfg(unix)]
fn make_private(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    let Ok(meta) = fs::metadata(path) else { return };
    if meta.permissions().mode() & 0o777 != mode {
        if let Err(e) = fs::set_permissions(path, fs::Permissions::from_mode(mode)) {
            log(&format!("Cannot restrict permissions of {}: {e}", path.display()));
        }
    }
}

pub fn config_path() -> PathBuf {
    app_dir().join("config.toml")
}

pub fn log_path() -> PathBuf {
    app_dir().join("log.txt")
}

/// Writes the commented template if no config exists yet.
/// Returns true on first run, i.e. only if the template was actually written.
pub fn ensure_exists() -> bool {
    let path = config_path();
    #[cfg(unix)]
    make_private(&app_dir(), 0o700);
    if path.exists() {
        #[cfg(unix)]
        make_private(&path, 0o600);
        return false;
    }
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600); // holds the token
    match options.open(&path).and_then(|mut f| f.write_all(TEMPLATE.as_bytes())) {
        Ok(()) => true,
        Err(e) => {
            log(&format!("Cannot create {}: {e}", path.display()));
            false
        }
    }
}

/// Last-modified time of config.toml, to notice when it is saved.
pub fn modified() -> Option<SystemTime> {
    fs::metadata(config_path()).and_then(|m| m.modified()).ok()
}

pub fn load() -> Result<FileConfig, String> {
    let text = fs::read_to_string(config_path()).map_err(|e| format!("cannot read config: {e}"))?;
    toml::from_str(&text).map_err(|e| format!("config.toml is invalid: {}", e.message()))
}

/// Holds an exclusive lock for the lifetime of the process so a second
/// copy (e.g. autostart + manual launch) exits instead of adding a 2nd icon.
/// `Ok(None)` means another copy is running; `Err` means the lock itself
/// could not be taken (e.g. a permissions problem).
pub fn acquire_instance_lock() -> io::Result<Option<File>> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(app_dir().join("instance.lock"))?;
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(TryLockError::Error(e)) => Err(e),
    }
}

/// Exists while first-run autostart setup still has to succeed, e.g. when
/// the first launch was from a temporary location on macOS.
fn autostart_pending_path() -> PathBuf {
    app_dir().join("autostart-pending")
}

pub fn autostart_pending() -> bool {
    autostart_pending_path().exists()
}

pub fn set_autostart_pending(pending: bool) {
    let path = autostart_pending_path();
    let result = if pending {
        fs::write(&path, "")
    } else {
        match fs::remove_file(&path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            other => other,
        }
    };
    if let Err(e) = result {
        log(&format!("Cannot update {}: {e}", path.display()));
    }
}

/// Both threads log; this keeps a rotation from racing with a write.
static LOG_LOCK: Mutex<()> = Mutex::new(());

/// Appends a timestamped line; rotates once the file passes 1 MB.
pub fn log(msg: &str) {
    let _guard = LOG_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let path = log_path();
    if fs::metadata(&path).is_ok_and(|m| m.len() > 1_000_000) {
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
