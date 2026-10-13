//! Headless DuckDNS updater for Docker / Linux servers.
//! Configured entirely through environment variables, logs to stdout.

use duckdns_core::{parse_domains, Outcome, Settings, Updater, MIN_INTERVAL, RETRY_AFTER_ERROR};
use std::time::Duration;

const DEFAULT_INTERVAL_SECS: u64 = 300;

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

/// DUCKDNS_INTERVAL in seconds; warns instead of silently replacing bad values.
fn interval_from_env() -> Duration {
    let raw = env_or("DUCKDNS_INTERVAL", &DEFAULT_INTERVAL_SECS.to_string());
    let secs = match raw.trim().parse::<u64>() {
        Ok(s) => s,
        Err(_) => {
            log(format!("warning: DUCKDNS_INTERVAL={raw:?} is not a number, using {DEFAULT_INTERVAL_SECS}"));
            DEFAULT_INTERVAL_SECS
        }
    };
    let interval = Duration::from_secs(secs);
    if interval < MIN_INTERVAL {
        log(format!("warning: DUCKDNS_INTERVAL={secs} is below the minimum, using {}", MIN_INTERVAL.as_secs()));
        return MIN_INTERVAL;
    }
    interval
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

    let interval = interval_from_env();

    // SIGINT/SIGTERM exit right away, even in the middle of a check:
    // an interrupted update is harmless and is simply redone next start.
    ctrlc::set_handler(|| {
        log("shutting down");
        std::process::exit(0);
    })
    .expect("failed to install signal handler");

    log(format!(
        "duckdns-daemon {} started: domains={} ipv6={} interval={}s",
        env!("CARGO_PKG_VERSION"),
        settings.domains.join(","),
        settings.ipv6,
        interval.as_secs()
    ));

    let mut updater = Updater::new(settings);
    let mut last_error = String::new();
    loop {
        let wait = match updater.run(false) {
            Ok(outcome) => {
                if !last_error.is_empty() {
                    log("recovered");
                    last_error.clear();
                }
                match outcome {
                    Outcome::Updated { ips, changed: true } => log(format!("updated DuckDNS -> {ips}")),
                    Outcome::Updated { ips, changed: false } => log(format!("DuckDNS already had {ips}")),
                    Outcome::Unchanged(_) => {} // quiet when nothing happens
                }
                interval
            }
            Err(e) => {
                let msg = e.to_string();
                if msg != last_error {
                    log(format!("error: {msg}"));
                    last_error = msg;
                }
                interval.min(RETRY_AFTER_ERROR)
            }
        };
        std::thread::sleep(wait);
    }
}
