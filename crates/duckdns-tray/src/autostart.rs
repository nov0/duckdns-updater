//! Start automatically when the user logs in.
//! Windows: HKCU\...\Run registry value. macOS: a LaunchAgent plist.
//! Both are per-user and need no admin rights.

pub use imp::*;

#[cfg(windows)]
mod imp {
    use std::io;
    use winreg::enums::{HKEY_CURRENT_USER, KEY_SET_VALUE};
    use winreg::RegKey;

    pub const MENU_LABEL: &str = "Start with Windows";
    const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const VALUE_NAME: &str = "DuckDNS Updater";

    pub fn is_enabled() -> bool {
        RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey(RUN_KEY)
            .and_then(|k| k.get_value::<String, _>(VALUE_NAME))
            .is_ok()
    }

    pub fn set(enabled: bool) -> io::Result<()> {
        let key = RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(RUN_KEY, KEY_SET_VALUE)?;
        if enabled {
            let exe = std::env::current_exe()?;
            key.set_value(VALUE_NAME, &format!("\"{}\"", exe.display()))
        } else {
            match key.delete_value(VALUE_NAME) {
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
                other => other,
            }
        }
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use std::fs;
    use std::io;
    use std::path::PathBuf;

    pub const MENU_LABEL: &str = "Start at login";
    const LABEL: &str = "org.duckdns.updater";

    fn plist_path() -> io::Result<PathBuf> {
        let home = std::env::var_os("HOME")
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME not set"))?;
        Ok(PathBuf::from(home).join("Library/LaunchAgents").join(format!("{LABEL}.plist")))
    }

    pub fn is_enabled() -> bool {
        plist_path().map_or(false, |p| p.exists())
    }

    /// Writes/removes the LaunchAgent. It takes effect at the next login;
    /// it is deliberately not loaded now, since the app is already running.
    pub fn set(enabled: bool) -> io::Result<()> {
        let path = plist_path()?;
        if !enabled {
            return match fs::remove_file(&path) {
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
                other => other,
            };
        }
        let exe = std::env::current_exe()?;
        let exe = xml_escape(&exe.to_string_lossy());
        let plist = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{exe}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>ProcessType</key>
    <string>Interactive</string>
</dict>
</plist>
"#
        );
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        fs::write(path, plist)
    }

    fn xml_escape(s: &str) -> String {
        s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
    }
}
