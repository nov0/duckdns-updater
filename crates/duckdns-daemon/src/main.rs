//! Headless DuckDNS updater for Docker / Linux servers.
//! Configured entirely through environment variables, logs to stdout.

use duckdns_core::{parse_domains, Outcome, Settings, Updater};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

fn env_bool(key: &str) -> bool {
    matches!(env_or(key, "false").to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on")
}

fn log(msg: impl AsRef<str>) {
    // Docker adds its own timestamps (`docker logs -t`), keep lines simple.
    println!("{}", msg.as_ref());
}

fn main() {
    let settings = Settings {
        domains: parse_domains(&env_or("DUCKDNS_DOMAINS", "")),
        token: env_or("DUCKDNS_TOKEN", ""),
        ipv6: env_bool("DUCKDNS_IPV6"),
    };
    if let Err(e) = settings.validate() {
        eprintln!("{e}. Set DUCKDNS_DOMAINS and DUCKDNS_TOKEN.");
        std::process::exit(2);
    }

    let interval_secs: u64 = env_or("DUCKDNS_INTERVAL", "300").parse().unwrap_or(300).max(30);
    let interval = Duration::from_secs(interval_secs);

    // Shutdown channel: SIGINT/SIGTERM drop the sender and wake the loop.
    let (tx, rx) = mpsc::channel::<()>();
    let mut tx = Some(tx);
    ctrlc::set_handler(move || {
        tx.take();
    })
    .expect("failed to install signal handler");

    log(format!(
        "duckdns-daemon {} started: domains={} ipv6={} interval={}s",
        env!("CARGO_PKG_VERSION"),
        settings.domains.join(","),
        settings.ipv6,
        interval_secs
    ));

    let mut updater = Updater::new(settings);
    loop {
        match updater.run(false) {
            Ok(Outcome::Updated { ips, changed: true }) => log(format!("updated DuckDNS -> {ips}")),
            Ok(Outcome::Updated { ips, changed: false }) => log(format!("DuckDNS already had {ips}")),
            Ok(Outcome::Unchanged(_)) => {} // quiet when nothing happens
            Err(e) => log(format!("error: {e}")),
        }

        match rx.recv_timeout(interval) {
            Err(RecvTimeoutError::Timeout) => continue,
            _ => break,
        }
    }
    log("shutting down");
}
