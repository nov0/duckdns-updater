# DuckDNS Updater (Rust)

One workspace, two apps sharing the same update logic:

| Crate | What it is |
|---|---|
| `duckdns-core` | Detects public IPv4/IPv6 and calls the DuckDNS update API |
| `duckdns-tray` | Tray app for Windows (system tray) and macOS (menu bar) |
| `duckdns-daemon` | Headless updater for Docker / Linux |

DuckDNS is only called when your IP changes, plus a forced refresh once a day.
The token is redacted from all log output.

## Windows tray app

```powershell
cargo build --release -p duckdns-tray
.\target\release\duckdns-tray.exe
```

On first launch it:
1. creates `%APPDATA%\DuckDNS Updater\config.toml` and opens it in Notepad,
2. registers itself to start with Windows (current user, no admin needed).

Fill in `domains` and `token`, save, then right-click the tray icon → **Update now**
(or double-click the icon).

Icon colour: green = up to date, amber = checking (at startup or after "Update now"), red = error (hover for details).

Menu: status line · Update now · Edit settings… · Open log · Start with Windows · Quit

Only one copy runs at a time. Config edits are picked up on the next check without
restarting. Logs go to `%APPDATA%\DuckDNS Updater\log.txt` (rotated at 1 MB).

Tip: Windows 11 hides new tray icons in the `^` overflow. Drag it onto the taskbar,
or enable it under Settings → Personalization → Taskbar → Other system tray icons.

## macOS menu-bar app

```bash
./scripts/bundle-macos.sh              # or --universal for Intel + Apple Silicon
mv "target/DuckDNS Updater.app" /Applications/
open "/Applications/DuckDNS Updater.app"
```

It behaves like the Windows version, with these differences:

- It lives only in the menu bar (no Dock icon).
- Settings are in `~/Library/Application Support/DuckDNS Updater/config.toml` and open in TextEdit.
- **Start at login** writes `~/Library/LaunchAgents/org.duckdns.updater.plist`.
  macOS will show a "background item added" notice. You can manage it under
  System Settings → General → Login Items.
- Click the icon to open the menu (there is no double-click on macOS).

Move the app to /Applications *before* first launch. If you move it later, it
re-points the login item on its next start.

The app is ad-hoc signed, which is fine for an app you built yourself. If you copy
the .app to another Mac (e.g. the CI artifact), Gatekeeper will block it the first
time. Right-click → Open, or run `xattr -dr com.apple.quarantine "/Applications/DuckDNS Updater.app"`.

## Docker

```bash
cp .env.example .env    # fill in domains + token
docker compose up -d --build
docker logs -f duckdns
```

Or without compose:

```bash
docker build -t duckdns-daemon .
docker run -d --name duckdns --restart unless-stopped \
  -e DUCKDNS_DOMAINS=myhome -e DUCKDNS_TOKEN=xxxxxxxx \
  duckdns-daemon
```

| Variable | Default | Meaning |
|---|---|---|
| `DUCKDNS_DOMAINS` | — | Comma-separated subdomains (with or without `.duckdns.org`) |
| `DUCKDNS_TOKEN` | — | Your DuckDNS token |
| `DUCKDNS_INTERVAL` | `300` | Seconds between checks (min 60) |
| `DUCKDNS_IPV6` | `false` | Also publish an AAAA record |

The image is `FROM scratch` (only the static binary, a few MB) and runs as an
unprivileged user. `docker stop` exits immediately (SIGTERM is handled).

## How the IP is detected

IPv4: api.ipify.org → ipv4.icanhazip.com → v4.ident.me (first that answers wins).
IPv6: api6.ipify.org → ipv6.icanhazip.com → v6.ident.me.
