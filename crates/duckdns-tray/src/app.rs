//! Tray icon + menu on the main thread, IP checks on a worker thread.

use crate::{autostart, config, icon};
use duckdns_core::{Outcome, PublicIps, Updater, RETRY_AFTER_ERROR};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::Duration;
use tao::event::{Event, StartCause};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{MouseButton, TrayIcon, TrayIconBuilder, TrayIconEvent};

enum UserEvent {
    Menu(MenuEvent),
    Tray(TrayIconEvent),
    Status(Status),
}

struct Status {
    state: icon::State,
    /// Shown as the first (greyed-out) menu line.
    headline: String,
    tooltip: String,
}

pub fn run() {
    std::panic::set_hook(Box::new(|info| config::log(&format!("PANIC: {info}"))));

    // Keep the lock alive until the process exits.
    let Some(_instance_lock) = config::acquire_instance_lock() else {
        return; // already running
    };

    let first_run = config::ensure_exists();
    if first_run || autostart::is_enabled() {
        // On later runs this re-points the entry in case the app was moved.
        match autostart::set(true) {
            Ok(()) if first_run => config::log("First run: created config.toml and enabled autostart"),
            Ok(()) => {}
            Err(e) => config::log(&format!("Could not set up autostart: {e}")),
        }
    }
    if first_run {
        config::open_in_editor(&config::config_path());
    }

    #[allow(unused_mut)]
    let mut event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();

    // macOS: menu-bar only, no Dock icon / app switcher entry.
    #[cfg(target_os = "macos")]
    {
        use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};
        event_loop.set_activation_policy(ActivationPolicy::Accessory);
    }

    let proxy = event_loop.create_proxy();
    MenuEvent::set_event_handler(Some(move |e| {
        let _ = proxy.send_event(UserEvent::Menu(e));
    }));
    let proxy = event_loop.create_proxy();
    TrayIconEvent::set_event_handler(Some(move |e| {
        let _ = proxy.send_event(UserEvent::Tray(e));
    }));

    // Sending () on this channel means "check now, and push even if unchanged".
    let (update_now, update_rx) = mpsc::channel::<()>();
    let proxy = event_loop.create_proxy();
    thread::spawn(move || worker(update_rx, proxy));

    let status_item = MenuItem::new("Starting…", false, None);
    let update_item = MenuItem::new("Update now", true, None);
    let settings_item = MenuItem::new("Edit settings…", true, None);
    let log_item = MenuItem::new("Open log", true, None);
    let autostart_item = CheckMenuItem::new(autostart::MENU_LABEL, true, autostart::is_enabled(), None);
    let quit_item = MenuItem::new("Quit", true, None);

    let menu = Menu::new();
    menu.append_items(&[
        &status_item,
        &PredefinedMenuItem::separator(),
        &update_item,
        &settings_item,
        &log_item,
        &PredefinedMenuItem::separator(),
        &autostart_item,
        &PredefinedMenuItem::separator(),
        &quit_item,
    ])
    .expect("failed to build tray menu");

    let mut menu = Some(menu);
    let mut tray = None;

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        match event {
            Event::NewEvents(StartCause::Init) => {
                let built = TrayIconBuilder::new()
                    .with_menu(Box::new(menu.take().expect("menu used once")))
                    .with_tooltip("DuckDNS Updater")
                    .with_icon(icon::make(icon::State::Working))
                    .build();
                match built {
                    Ok(t) => tray = Some(t),
                    Err(e) => {
                        config::log(&format!("Cannot create tray icon: {e}"));
                        *control_flow = ControlFlow::Exit;
                    }
                }
            }

            Event::UserEvent(UserEvent::Status(s)) => {
                status_item.set_text(&s.headline);
                if let Some(t) = &tray {
                    let _ = t.set_icon(Some(icon::make(s.state)));
                    let _ = t.set_tooltip(Some(truncate(&s.tooltip, 120)));
                }
            }

            // Windows only; on macOS a click simply opens the menu.
            Event::UserEvent(UserEvent::Tray(TrayIconEvent::DoubleClick {
                button: MouseButton::Left,
                ..
            })) => {
                show_checking(&status_item, tray.as_ref());
                let _ = update_now.send(());
            }

            Event::UserEvent(UserEvent::Menu(e)) => {
                if e.id == update_item.id() {
                    show_checking(&status_item, tray.as_ref());
                    let _ = update_now.send(());
                } else if e.id == settings_item.id() {
                    config::open_in_editor(&config::config_path());
                } else if e.id == log_item.id() {
                    config::open_in_editor(&config::log_path());
                } else if e.id == autostart_item.id() {
                    let want = !autostart::is_enabled();
                    if let Err(err) = autostart::set(want) {
                        config::log(&format!("Could not change autostart: {err}"));
                    }
                    // Reflect the real registry state, whatever happened.
                    autostart_item.set_checked(autostart::is_enabled());
                } else if e.id == quit_item.id() {
                    tray.take(); // removes the icon immediately
                    *control_flow = ControlFlow::Exit;
                }
            }

            _ => {}
        }
    });
}

/// Background loop: re-reads the config every cycle (so edits apply without
/// a restart), checks the IP, and reports status to the UI thread.
fn worker(update_now: Receiver<()>, ui: EventLoopProxy<UserEvent>) {
    let mut updater: Option<Updater> = None;
    let mut force = true; // always push once at startup
    let mut last_error = String::new();

    loop {
        let mut wait = Duration::from_secs(300);

        let result = config::load().and_then(|cfg| {
            wait = cfg.interval();
            let settings = cfg.settings();
            let up = updater.get_or_insert_with(|| Updater::new(settings.clone()));
            up.set_settings(settings);
            let domains = up.settings().domains.join(", ");
            up.run(force).map(|o| (o, domains)).map_err(|e| e.to_string())
        });

        let status = match result {
            Ok((outcome, domains)) => {
                last_error.clear();
                let ips = match outcome {
                    Outcome::Updated { ips, changed: true } => {
                        config::log(&format!("Updated {domains} -> {ips}"));
                        ips
                    }
                    Outcome::Updated { ips, changed: false } => {
                        config::log(&format!("DuckDNS already had {ips} for {domains}"));
                        ips
                    }
                    Outcome::Unchanged(ips) => ips,
                };
                ok_status(&ips, &domains)
            }
            Err(msg) => {
                if msg != last_error {
                    config::log(&format!("Error: {msg}"));
                    last_error = msg.clone();
                }
                wait = wait.min(RETRY_AFTER_ERROR);
                Status {
                    state: icon::State::Error,
                    headline: format!("⚠ {msg}"),
                    tooltip: format!("DuckDNS Updater\n{msg}"),
                }
            }
        };

        if ui.send_event(UserEvent::Status(status)).is_err() {
            return; // UI is gone
        }

        force = false;
        match update_now.recv_timeout(wait) {
            Ok(()) => {
                force = true;
                // Several clicks while busy mean one check, not one each.
                while update_now.try_recv().is_ok() {}
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// Immediate feedback for a manual check; the worker reports the result.
fn show_checking(status_item: &MenuItem, tray: Option<&TrayIcon>) {
    status_item.set_text("Checking…");
    if let Some(t) = tray {
        let _ = t.set_icon(Some(icon::make(icon::State::Working)));
    }
}

fn ok_status(ips: &PublicIps, domains: &str) -> Status {
    let time = chrono::Local::now().format("%H:%M");
    Status {
        state: icon::State::Ok,
        headline: format!("IP {ips}  (checked {time})"),
        tooltip: format!("DuckDNS: {domains}\nIP: {ips}\nChecked {time}"),
    }
}

/// Windows limits tray tooltips to 128 UTF-16 units (harmless on macOS).
fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        s.chars().take(max_chars - 1).chain(['…']).collect()
    }
}
