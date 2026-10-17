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
/// IP lookups have fallbacks, so a slow provider is skipped sooner.
const LOOKUP_TIMEOUT: Duration = Duration::from_secs(5);

/// After a failure, retry sooner than the normal interval
/// (helps right after boot, before the network is up).
pub const RETRY_AFTER_ERROR: Duration = Duration::from_secs(60);
/// Shortest allowed check interval, shared by both apps.
pub const MIN_INTERVAL: Duration = Duration::from_secs(60);
/// Pauses between attempts of the DuckDNS request. DuckDNS's own name
/// servers are flaky: a lookup of www.duckdns.org often fails once and
/// works a few seconds later.
const SEND_RETRY_DELAYS: [Duration; 2] = [Duration::from_secs(2), Duration::from_secs(5)];

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

#[derive(Clone, PartialEq, Eq)]
pub struct Settings {
    /// Subdomain names only, e.g. `["myhome"]` for myhome.duckdns.org.
    pub domains: Vec<String>,
    pub token: String,
    /// Also detect and publish an IPv6 (AAAA) address.
    pub ipv6: bool,
}

/// Hand-written so the token never shows up in a debug print.
impl fmt::Debug for Settings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Settings")
            .field("domains", &self.domains)
            .field("token", &"***")
            .field("ipv6", &self.ipv6)
            .finish()
    }
}

impl Settings {
    pub fn validate(&self) -> Result<(), Error> {
        if self.domains.is_empty() {
            return Err(Error::Config("no domains configured".into()));
        }
        for (i, d) in self.domains.iter().enumerate() {
            if !is_valid_subdomain(d) {
                return Err(Error::Config(format!(
                    "{d:?} is not a valid DuckDNS subdomain (use letters, digits and '-', e.g. \"myhome\")"
                )));
            }
            if self.domains[..i].contains(d) {
                return Err(Error::Config(format!("{d:?} is listed twice")));
            }
        }
        let t = self.token.trim();
        if t.is_empty() || t.contains("your-token") {
            return Err(Error::Config("DuckDNS token is not set".into()));
        }
        Ok(())
    }
}

/// One DNS label: 1-63 letters, digits or '-', not starting or ending with '-'.
fn is_valid_subdomain(d: &str) -> bool {
    (1..=63).contains(&d.len())
        && d.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        && !d.starts_with('-')
        && !d.ends_with('-')
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

/// Where requests go. Only tests change these.
struct Endpoints {
    update: String,
    v4: Vec<String>,
    v6: Vec<String>,
    retry_delays: Vec<Duration>,
}

impl Default for Endpoints {
    fn default() -> Self {
        let owned = |list: &[&str]| list.iter().map(|s| s.to_string()).collect();
        Self {
            update: "https://www.duckdns.org/update".into(),
            v4: owned(IPV4_PROVIDERS),
            v6: owned(IPV6_PROVIDERS),
            retry_delays: SEND_RETRY_DELAYS.to_vec(),
        }
    }
}

pub struct Updater {
    agent: ureq::Agent,
    endpoints: Endpoints,
    settings: Settings,
    last_sent: Option<PublicIps>,
    last_success: Option<Instant>,
}

impl Updater {
    pub fn new(settings: Settings) -> Self {
        Self::with_endpoints(settings, Endpoints::default())
    }

    fn with_endpoints(settings: Settings, endpoints: Endpoints) -> Self {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .user_agent(concat!("duckdns-updater/", env!("CARGO_PKG_VERSION")))
            .build()
            .into();
        Self { agent, endpoints, settings, last_sent: None, last_success: None }
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
        let detected = self.detect_ips()?;
        let known = merge_with_last(&detected, self.last_sent.as_ref())?;

        let stale = self
            .last_success
            .is_none_or(|t| t.elapsed() >= FORCE_REFRESH_AFTER);
        if !force && !stale && self.last_sent.as_ref() == Some(&known) {
            return Ok(Outcome::Unchanged(known));
        }

        // Only what was detected now is sent; DuckDNS keeps the rest as is.
        let changed = self.send(&detected)?;
        self.last_sent = Some(known.clone());
        self.last_success = Some(Instant::now());
        Ok(Outcome::Updated { ips: known, changed })
    }

    /// IPv4 and IPv6 are looked up at the same time, so a dead family
    /// doesn't add its timeouts on top of the other one.
    fn detect_ips(&self) -> Result<PublicIps, Error> {
        let (v4, v6) = std::thread::scope(|s| {
            let v6 = self
                .settings
                .ipv6
                .then(|| s.spawn(|| self.first_answer::<Ipv6Addr>(&self.endpoints.v6)));
            let v4 = self.first_answer::<Ipv4Addr>(&self.endpoints.v4);
            (v4, v6.and_then(|h| h.join().ok().flatten()))
        });
        if v4.is_none() && v6.is_none() {
            return Err(Error::NoIp);
        }
        Ok(PublicIps { v4, v6 })
    }

    /// Ask each provider in turn; return the first answer that parses as `T`.
    fn first_answer<T: std::str::FromStr>(&self, providers: &[String]) -> Option<T> {
        providers.iter().find_map(|url| {
            let body = self.get(url, LOOKUP_TIMEOUT).ok()?;
            body.trim().parse::<T>().ok()
        })
    }

    /// Returns whether DuckDNS reported an actual change.
    fn send(&self, ips: &PublicIps) -> Result<bool, Error> {
        let domains: Vec<String> = self.settings.domains.iter().map(|d| url_encode(d)).collect();
        let mut url = format!(
            "{}?domains={}&token={}&verbose=true",
            self.endpoints.update,
            domains.join(","),
            url_encode(self.settings.token.trim())
        );
        if let Some(v4) = ips.v4 {
            url.push_str(&format!("&ip={v4}"));
        }
        if let Some(v6) = ips.v6 {
            url.push_str(&format!("&ipv6={v6}"));
        }

        // verbose=true answers e.g. "OK\n1.2.3.4\n\nUPDATED" or "...NOCHANGE"
        let body = self.get_retrying(&url)?;
        let mut lines = body.lines().map(str::trim);
        match lines.next() {
            Some("OK") => Ok(body.contains("UPDATED")),
            _ => Err(Error::Rejected),
        }
    }

    /// Like `get`, but retries network errors (not answers like "KO").
    fn get_retrying(&self, url: &str) -> Result<String, Error> {
        let mut result = self.get(url, HTTP_TIMEOUT);
        for &delay in &self.endpoints.retry_delays {
            if !matches!(result, Err(Error::Http(_))) {
                break;
            }
            std::thread::sleep(delay);
            result = self.get(url, HTTP_TIMEOUT);
        }
        result
    }

    fn get(&self, url: &str, timeout: Duration) -> Result<String, Error> {
        let mut resp = self
            .agent
            .get(url)
            .config()
            .timeout_global(Some(timeout))
            .build()
            .call()
            .map_err(|e| Error::Http(self.redact(&e.to_string())))?;
        resp.body_mut()
            .read_to_string()
            .map_err(|e| Error::Http(self.redact(&e.to_string())))
    }

    /// Never let the token end up in a log file or tooltip.
    fn redact(&self, msg: &str) -> String {
        let token = self.settings.token.trim();
        if token.is_empty() {
            return msg.to_string();
        }
        // The URL carries the percent-encoded form, so hide that one too.
        msg.replace(&url_encode(token), "***").replace(token, "***")
    }
}

/// Fills gaps in a fresh detection from what was last sent, so a lookup
/// that briefly fails doesn't look like an IP change.
///
/// A missing IPv6 just keeps the previous one (DuckDNS leaves AAAA alone
/// when `ipv6=` is omitted). A missing IPv4 is an error if we had one
/// before: omitting `ip=` makes DuckDNS guess it from the request's source
/// address, which can be wrong. IPv6-only hosts never had an IPv4, so they
/// still update.
fn merge_with_last(detected: &PublicIps, last: Option<&PublicIps>) -> Result<PublicIps, Error> {
    let last_v4 = last.and_then(|l| l.v4);
    if detected.v4.is_none() && last_v4.is_some() {
        return Err(Error::NoIp);
    }
    Ok(PublicIps {
        v4: detected.v4,
        v6: detected.v6.or(last.and_then(|l| l.v6)),
    })
}

/// Percent-encodes everything except RFC 3986 unreserved characters.
fn url_encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ips(v4: Option<&str>, v6: Option<&str>) -> PublicIps {
        PublicIps {
            v4: v4.map(|s| s.parse().unwrap()),
            v6: v6.map(|s| s.parse().unwrap()),
        }
    }

    #[test]
    fn missing_ipv6_keeps_the_previous_one() {
        let last = ips(Some("1.2.3.4"), Some("2001:db8::1"));
        let merged = merge_with_last(&ips(Some("1.2.3.4"), None), Some(&last)).unwrap();
        assert_eq!(merged, last);
    }

    #[test]
    fn missing_ipv4_after_having_one_is_an_error() {
        let last = ips(Some("1.2.3.4"), Some("2001:db8::1"));
        assert!(merge_with_last(&ips(None, Some("2001:db8::1")), Some(&last)).is_err());
    }

    #[test]
    fn ipv6_only_host_still_updates() {
        let only_v6 = ips(None, Some("2001:db8::1"));
        assert_eq!(merge_with_last(&only_v6, None).unwrap(), only_v6);
        assert_eq!(merge_with_last(&only_v6, Some(&only_v6)).unwrap(), only_v6);
    }

    #[test]
    fn url_encoding() {
        assert_eq!(url_encode("abc-123_x.y~"), "abc-123_x.y~");
        assert_eq!(url_encode("a&b c=d"), "a%26b%20c%3Dd");
    }

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

    fn settings(domains: &[&str]) -> Settings {
        Settings {
            domains: domains.iter().map(|d| d.to_string()).collect(),
            token: "tok".into(),
            ipv6: false,
        }
    }

    #[test]
    fn bad_domains_are_rejected() {
        assert!(settings(&["home", "my-nas2"]).validate().is_ok());
        for bad in ["my home", "-home", "home-", "www.home", "a_b", &"x".repeat(64)] {
            assert!(settings(&[bad]).validate().is_err(), "{bad:?} should be rejected");
        }
        assert!(settings(&["home", "home"]).validate().is_err());
    }

    #[test]
    fn debug_hides_the_token() {
        let s = Settings { token: "secret-token".into(), ..settings(&["home"]) };
        assert!(!format!("{s:?}").contains("secret-token"));
    }

    #[test]
    fn redact_covers_the_encoded_token() {
        let up = Updater::new(Settings { token: "a&b c".into(), ..settings(&["home"]) });
        let msg = "failed: token=a%26b%20c and a&b c";
        assert_eq!(up.redact(msg), "failed: token=*** and ***");
    }

    // --- Updater against a local fake server ---

    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    type Reply = (u16, &'static str);

    /// Answers /v4, /v6 and /update with scripted replies and records
    /// every /update query.
    struct Mock {
        v4: Mutex<Reply>,
        v6: Mutex<Reply>,
        /// Used in order; the last one repeats.
        update: Mutex<Vec<Reply>>,
        updates: Mutex<Vec<String>>,
    }

    const UPDATED: Reply = (200, "OK\n1.2.3.4\n\nUPDATED");
    const NOCHANGE: Reply = (200, "OK\n1.2.3.4\n\nNOCHANGE");
    const DOWN: Reply = (500, "");

    impl Mock {
        fn set_v4(&self, r: Reply) {
            *self.v4.lock().unwrap() = r;
        }
        fn set_v6(&self, r: Reply) {
            *self.v6.lock().unwrap() = r;
        }
        fn set_update(&self, replies: &[Reply]) {
            *self.update.lock().unwrap() = replies.to_vec();
        }
        fn updates(&self) -> Vec<String> {
            self.updates.lock().unwrap().clone()
        }

        fn reply(&self, path: &str) -> Reply {
            if path.starts_with("/v4") {
                *self.v4.lock().unwrap()
            } else if path.starts_with("/v6") {
                *self.v6.lock().unwrap()
            } else if path.starts_with("/update") {
                self.updates.lock().unwrap().push(path.to_string());
                let mut queue = self.update.lock().unwrap();
                if queue.len() > 1 { queue.remove(0) } else { queue[0] }
            } else {
                (404, "")
            }
        }
    }

    fn mock_updater(s: Settings) -> (Updater, Arc<Mock>) {
        let mock = Arc::new(Mock {
            v4: Mutex::new((200, "1.2.3.4")),
            v6: Mutex::new((200, "2001:db8::1")),
            update: Mutex::new(vec![UPDATED]),
            updates: Mutex::default(),
        });
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = Arc::clone(&mock);
        std::thread::spawn(move || {
            for mut stream in listener.incoming().flatten() {
                let mut buf = Vec::new();
                let mut chunk = [0; 1024];
                while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    match stream.read(&mut chunk) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => buf.extend_from_slice(&chunk[..n]),
                    }
                }
                let head = String::from_utf8_lossy(&buf);
                let path = head.split_whitespace().nth(1).unwrap_or("").to_string();
                let (status, body) = server.reply(&path);
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        let endpoints = Endpoints {
            update: format!("{base}/update"),
            v4: vec![format!("{base}/v4")],
            v6: vec![format!("{base}/v6")],
            retry_delays: vec![Duration::ZERO; 2],
        };
        (Updater::with_endpoints(s, endpoints), mock)
    }

    #[test]
    fn unchanged_ip_is_not_resent() {
        let (mut up, mock) = mock_updater(settings(&["home"]));
        assert!(matches!(up.run(false), Ok(Outcome::Updated { changed: true, .. })));
        assert!(matches!(up.run(false), Ok(Outcome::Unchanged(_))));
        assert_eq!(mock.updates().len(), 1);
        let q = &mock.updates()[0];
        assert!(q.contains("domains=home") && q.contains("token=tok") && q.contains("ip=1.2.3.4"));
        assert!(!q.contains("ipv6="));
    }

    #[test]
    fn force_and_daily_refresh_resend() {
        let (mut up, mock) = mock_updater(settings(&["home"]));
        up.run(false).unwrap();
        mock.set_update(&[NOCHANGE]);
        assert!(matches!(up.run(true), Ok(Outcome::Updated { changed: false, .. })));

        up.last_success = Instant::now().checked_sub(FORCE_REFRESH_AFTER);
        assert!(matches!(up.run(false), Ok(Outcome::Updated { .. })));
        assert_eq!(mock.updates().len(), 3);
    }

    #[test]
    fn new_ip_or_new_settings_resend() {
        let (mut up, mock) = mock_updater(settings(&["home"]));
        up.run(false).unwrap();
        mock.set_v4((200, "5.6.7.8"));
        assert!(matches!(up.run(false), Ok(Outcome::Updated { .. })));
        assert!(mock.updates()[1].contains("ip=5.6.7.8"));

        up.set_settings(settings(&["home", "office"]));
        assert!(matches!(up.run(false), Ok(Outcome::Updated { .. })));
        assert!(mock.updates()[2].contains("domains=home,office"));
    }

    #[test]
    fn network_errors_are_retried() {
        let (mut up, mock) = mock_updater(settings(&["home"]));
        mock.set_update(&[DOWN, DOWN, UPDATED]);
        assert!(up.run(false).is_ok());
        assert_eq!(mock.updates().len(), 3);
    }

    #[test]
    fn ko_is_not_retried() {
        let (mut up, mock) = mock_updater(settings(&["home"]));
        mock.set_update(&[(200, "KO")]);
        assert!(matches!(up.run(false), Err(Error::Rejected)));
        assert_eq!(mock.updates().len(), 1);
    }

    #[test]
    fn failed_update_is_not_cached() {
        let (mut up, mock) = mock_updater(settings(&["home"]));
        mock.set_update(&[DOWN, DOWN, DOWN, UPDATED]);
        assert!(matches!(up.run(false), Err(Error::Http(_))));
        assert!(matches!(up.run(false), Ok(Outcome::Updated { .. })));
        assert_eq!(mock.updates().len(), 4);
    }

    #[test]
    fn failed_ipv6_lookup_keeps_the_previous_one() {
        let (mut up, mock) = mock_updater(Settings { ipv6: true, ..settings(&["home"]) });
        up.run(false).unwrap();
        assert!(mock.updates()[0].contains("ipv6=2001:db8::1"));

        mock.set_v6(DOWN);
        let known = ips(Some("1.2.3.4"), Some("2001:db8::1"));
        assert_eq!(up.run(false).unwrap(), Outcome::Unchanged(known.clone()));
        // A forced send leaves IPv6 out, so DuckDNS keeps the old AAAA record.
        assert_eq!(up.run(true).unwrap(), Outcome::Updated { ips: known, changed: true });
        assert!(!mock.updates()[1].contains("ipv6="));
    }

    #[test]
    fn failed_ipv4_lookup_after_having_one_sends_nothing() {
        let (mut up, mock) = mock_updater(Settings { ipv6: true, ..settings(&["home"]) });
        up.run(false).unwrap();
        mock.set_v4(DOWN);
        assert!(matches!(up.run(true), Err(Error::NoIp)));
        assert_eq!(mock.updates().len(), 1);
    }

    #[test]
    fn no_ip_at_all_is_an_error() {
        let (mut up, mock) = mock_updater(settings(&["home"]));
        mock.set_v4((200, "not an ip"));
        assert!(matches!(up.run(false), Err(Error::NoIp)));
        assert!(mock.updates().is_empty());
    }
}
