//! Shared logic for both apps: detect the public IP and push it to DuckDNS.
//!
//! The [`Updater`] remembers what it last sent, so callers can poll often
//! and DuckDNS is only contacted when the IP actually changes (plus a
//! forced refresh once a day as a safety net).

use std::fmt;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::time::{Duration, Instant};

/// Re-send even an unchanged IP after this long, in case the record was
/// changed elsewhere (e.g. by hand on the DuckDNS website).
const FORCE_REFRESH_AFTER: Duration = Duration::from_secs(24 * 60 * 60);
const HTTP_TIMEOUT: Duration = Duration::from_secs(15);

const IPV4_PROVIDERS: &[&str] = &[
    "https://api.ipify.org",
    "https://ipv4.icanhazip.com",
    "https://v4.ident.me",
];
const IPV6_PROVIDERS: &[&str] = &[
    "https://api6.ipify.org",
    "https://ipv6.icanhazip.com",
    "https://v6.ident.me",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// Subdomain names only, e.g. `["myhome"]` for myhome.duckdns.org.
    pub domains: Vec<String>,
    pub token: String,
    /// Also detect and publish an IPv6 (AAAA) address.
    pub ipv6: bool,
}

impl Settings {
    pub fn validate(&self) -> Result<(), Error> {
        if self.domains.is_empty() {
            return Err(Error::Config("no domains configured".into()));
        }
        let t = self.token.trim();
        if t.is_empty() || t.contains("your-token") {
            return Err(Error::Config("DuckDNS token is not set".into()));
        }
        Ok(())
    }
}

/// Accepts `"a, b.duckdns.org ,c"` and returns `["a", "b", "c"]`.
pub fn parse_domains(input: &str) -> Vec<String> {
    input
        .split(',')
        .map(|d| d.trim().to_ascii_lowercase())
        .map(|d| d.trim_end_matches(".duckdns.org").to_string())
        .filter(|d| !d.is_empty())
        .collect()
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PublicIps {
    pub v4: Option<Ipv4Addr>,
    pub v6: Option<Ipv6Addr>,
}

impl fmt::Display for PublicIps {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.v4, self.v6) {
            (Some(a), Some(b)) => write!(f, "{a} / {b}"),
            (Some(a), None) => write!(f, "{a}"),
            (None, Some(b)) => write!(f, "{b}"),
            (None, None) => write!(f, "unknown"),
        }
    }
}

#[derive(Debug)]
pub enum Error {
    Config(String),
    /// None of the IP lookup services answered.
    NoIp,
    Http(String),
    /// DuckDNS answered "KO" — almost always a wrong token or domain.
    Rejected,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Config(m) => write!(f, "config error: {m}"),
            Error::NoIp => write!(f, "could not detect public IP (offline?)"),
            Error::Http(m) => write!(f, "network error: {m}"),
            Error::Rejected => write!(f, "DuckDNS rejected the update (check token/domains)"),
        }
    }
}

impl std::error::Error for Error {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// DuckDNS was called. `changed` is false when DuckDNS already had this IP.
    Updated { ips: PublicIps, changed: bool },
    /// IP matches what we last sent; DuckDNS was not contacted.
    Unchanged(PublicIps),
}

pub struct Updater {
    agent: ureq::Agent,
    settings: Settings,
    last_sent: Option<PublicIps>,
    last_success: Option<Instant>,
}

impl Updater {
    pub fn new(settings: Settings) -> Self {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(HTTP_TIMEOUT))
            .user_agent(concat!("duckdns-updater/", env!("CARGO_PKG_VERSION")))
            .build()
            .into();
        Self { agent, settings, last_sent: None, last_success: None }
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// Swap in new settings (e.g. after the config file was edited).
    /// Forces the next run to push to DuckDNS if anything relevant changed.
    pub fn set_settings(&mut self, settings: Settings) {
        if settings != self.settings {
            self.settings = settings;
            self.last_sent = None;
        }
    }

    /// Detect the public IP and update DuckDNS if needed.
    /// `force` skips the "unchanged" shortcut.
    pub fn run(&mut self, force: bool) -> Result<Outcome, Error> {
        self.settings.validate()?;
        let ips = self.detect_ips()?;

        let stale = self
            .last_success
            .map_or(true, |t| t.elapsed() >= FORCE_REFRESH_AFTER);
        if !force && !stale && self.last_sent.as_ref() == Some(&ips) {
            return Ok(Outcome::Unchanged(ips));
        }

        let changed = self.send(&ips)?;
        self.last_sent = Some(ips.clone());
        self.last_success = Some(Instant::now());
        Ok(Outcome::Updated { ips, changed })
    }

    fn detect_ips(&self) -> Result<PublicIps, Error> {
        let v4 = self.first_answer(IPV4_PROVIDERS);
        let v6 = if self.settings.ipv6 { self.first_answer(IPV6_PROVIDERS) } else { None };
        if v4.is_none() && v6.is_none() {
            return Err(Error::NoIp);
        }
        Ok(PublicIps { v4, v6 })
    }

    /// Ask each provider in turn; return the first answer that parses as `T`.
    fn first_answer<T: std::str::FromStr>(&self, providers: &[&str]) -> Option<T> {
        providers.iter().find_map(|url| {
            let body = self.get(url).ok()?;
            body.trim().parse::<T>().ok()
        })
    }

    /// Returns whether DuckDNS reported an actual change.
    fn send(&self, ips: &PublicIps) -> Result<bool, Error> {
        let mut url = format!(
            "https://www.duckdns.org/update?domains={}&token={}&verbose=true",
            self.settings.domains.join(","),
            self.settings.token.trim()
        );
        if let Some(v4) = ips.v4 {
            url.push_str(&format!("&ip={v4}"));
        }
        if let Some(v6) = ips.v6 {
            url.push_str(&format!("&ipv6={v6}"));
        }

        // verbose=true answers e.g. "OK\n1.2.3.4\n\nUPDATED" or "...NOCHANGE"
        let body = self.get(&url)?;
        let mut lines = body.lines().map(str::trim);
        match lines.next() {
            Some("OK") => Ok(body.contains("UPDATED")),
            _ => Err(Error::Rejected),
        }
    }

    fn get(&self, url: &str) -> Result<String, Error> {
        let mut resp = self
            .agent
            .get(url)
            .call()
            .map_err(|e| Error::Http(self.redact(&e.to_string())))?;
        resp.body_mut()
            .read_to_string()
            .map_err(|e| Error::Http(self.redact(&e.to_string())))
    }

    /// Never let the token end up in a log file or tooltip.
    fn redact(&self, msg: &str) -> String {
        let token = self.settings.token.trim();
        if token.is_empty() { msg.to_string() } else { msg.replace(token, "***") }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domains_are_normalised() {
        assert_eq!(
            parse_domains(" Home, office.duckdns.org ,, nas "),
            vec!["home", "office", "nas"]
        );
    }

    #[test]
    fn placeholder_token_is_rejected() {
        let s = Settings {
            domains: vec!["x".into()],
            token: "your-token-here".into(),
            ipv6: false,
        };
        assert!(s.validate().is_err());
    }
}
