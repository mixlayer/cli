use std::fs;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use anyhow::Context;
use reqwest::blocking::Client as HttpClient;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::api::config::Config;
use crate::paths::{self, Kind};

#[derive(Debug)]
pub struct ApiError {
    pub method: String,
    pub path: String,
    pub status: u16,
    pub body: String,
    pub retry_after: String,
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} {} returned HTTP {}: {}",
            self.method, self.path, self.status, self.body
        )
    }
}
impl std::error::Error for ApiError {}

#[derive(Debug)]
struct Limiter {
    rate: f64,
    floor: f64,
    ceiling: f64,
    successes: usize,
    last: Option<Instant>,
}

impl Limiter {
    fn new(rate: f64) -> Self {
        Self {
            rate,
            floor: rate.min(0.5),
            ceiling: 0.0,
            successes: 0,
            last: None,
        }
    }
    fn wait(&mut self) {
        if self.rate <= 0.0 {
            return;
        }
        let Ok(interval) = Duration::try_from_secs_f64(1.0 / self.rate) else {
            return;
        };
        if let Some(last) = self.last
            && let Some(remaining) = interval.checked_sub(last.elapsed())
        {
            thread::sleep(remaining);
        }
        self.last = Some(Instant::now());
    }
    fn success(&mut self) {
        if self.rate <= 0.0 {
            return;
        }
        self.successes += 1;
        if self.successes >= 10 {
            self.rate = (self.rate * 1.25).max(self.floor);
            if self.ceiling > 0.0 {
                self.rate = self.rate.min(self.ceiling * 0.9);
            }
            self.successes = 0;
        }
    }
    fn throttle(&mut self) {
        if self.rate <= 0.0 {
            return;
        }
        self.ceiling = self.rate;
        self.rate = (self.rate / 2.0).max(self.floor);
        self.successes = 0;
    }
}

pub struct Client {
    pub config: Config,
    http: HttpClient,
    cache_dir: PathBuf,
    no_cache: bool,
    /// Shared by every clone, so parallel calls from one command respect
    /// one `--rate-limit` (as the Go client did).
    limiter: std::sync::Arc<std::sync::Mutex<Limiter>>,
}

pub fn parse_duration(raw: &str) -> anyhow::Result<Duration> {
    let mut rest = raw.trim();
    if rest.is_empty() {
        anyhow::bail!("empty duration");
    }
    if rest == "0" {
        return Ok(Duration::ZERO);
    }
    if rest.starts_with('-') {
        rest = &rest[1..];
    }
    let mut seconds = 0.0;
    while !rest.is_empty() {
        let n = rest
            .find(|c: char| !c.is_ascii_digit() && c != '.')
            .context("duration missing unit")?;
        let value: f64 = rest[..n].parse()?;
        rest = &rest[n..];
        let (unit, scale) = if let Some(s) = rest.strip_prefix("ns") {
            rest = s;
            ("ns", 0.000_000_001)
        } else if let Some(s) = rest
            .strip_prefix("us")
            .or_else(|| rest.strip_prefix("µs"))
            .or_else(|| rest.strip_prefix("μs"))
        {
            rest = s;
            ("us", 0.000_001)
        } else if let Some(s) = rest.strip_prefix("ms") {
            rest = s;
            ("ms", 0.001)
        } else if let Some(s) = rest.strip_prefix('h') {
            rest = s;
            ("h", 3600.0)
        } else if let Some(s) = rest.strip_prefix('m') {
            rest = s;
            ("m", 60.0)
        } else if let Some(s) = rest.strip_prefix('s') {
            rest = s;
            ("s", 1.0)
        } else {
            anyhow::bail!("invalid duration unit");
        };
        let _ = unit;
        seconds += value * scale;
    }
    if !seconds.is_finite() || seconds < 0.0 || seconds >= u64::MAX as f64 {
        anyhow::bail!("invalid duration");
    }
    Ok(if raw.trim().starts_with('-') {
        Duration::ZERO
    } else {
        Duration::from_secs_f64(seconds)
    })
}

impl Client {
    pub fn new(config: Config, timeout: Duration, rate: f64, no_cache: bool) -> anyhow::Result<Self> {
        let mut builder = HttpClient::builder();
        if !timeout.is_zero() {
            builder = builder.timeout(timeout);
        }
        let http = builder.build()?;
        Ok(Self {
            config,
            http,
            cache_dir: paths::dir(Kind::Cache)?.join("http"),
            no_cache,
            limiter: std::sync::Arc::new(std::sync::Mutex::new(Limiter::new(rate))),
        })
    }

    /// A client for another thread of the same command: same settings and
    /// the same rate limiter.
    pub fn worker(&self) -> Self {
        Self {
            config: self.config.clone(),
            http: self.http.clone(),
            cache_dir: self.cache_dir.clone(),
            no_cache: self.no_cache,
            limiter: self.limiter.clone(),
        }
    }

    fn limiter(&self) -> std::sync::MutexGuard<'_, Limiter> {
        self.limiter.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn cache_key(&self, path: &str) -> String {
        let mut key = format!(
            "{}|base_url={}|auth_source={}",
            path,
            self.config.base_url.trim_end_matches('/'),
            self.config.auth_source
        );
        if !self.config.auth_header.is_empty() {
            let h = Sha256::digest(self.config.auth_header.as_bytes());
            key.push_str(&format!("|auth={}", hex::encode(&h[..8])));
        }
        key.push_str(&format!("|config_path={}", self.config.path.display()));
        let hash = Sha256::digest(key.as_bytes());
        hex::encode(&hash[..8])
    }

    fn cached(&self, path: &str) -> Option<String> {
        if self.no_cache {
            return None;
        }
        let file = self.cache_dir.join(format!("{}.json", self.cache_key(path)));
        let age = fs::metadata(&file).ok()?.modified().ok()?.elapsed().ok()?;
        if age > Duration::from_secs(300) {
            return None;
        }
        fs::read_to_string(file).ok()
    }

    fn cache(&self, path: &str, value: &str) {
        if self.no_cache {
            return;
        }
        if crate::api::config::create_private_dir(&self.cache_dir).is_err() {
            return;
        }
        let file = self.cache_dir.join(format!("{}.json", self.cache_key(path)));
        let mut options = fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        if let Ok(mut output) = options.open(&file) {
            use std::io::Write;
            let _ = output.write_all(value.as_bytes());
            #[cfg(unix)]
            let _ = fs::set_permissions(&file, std::os::unix::fs::PermissionsExt::from_mode(0o600));
        }
    }

    pub fn get(&mut self, path: &str) -> anyhow::Result<Value> {
        let raw = self.get_raw(path)?;
        Ok(serde_json::from_str(&raw).unwrap_or(Value::String(raw)))
    }

    /// Keep the wire JSON for output modes that pass it through verbatim.
    pub fn get_raw(&mut self, path: &str) -> anyhow::Result<String> {
        if let Some(v) = self.cached(path) {
            return Ok(v);
        }
        let v = self.request_raw("GET", path, None, false)?;
        self.cache(path, &v);
        Ok(v)
    }

    pub fn get_no_cache(&mut self, path: &str) -> anyhow::Result<Value> {
        let v = self.request_raw("GET", path, None, false)?;
        self.cache(path, &v);
        Ok(serde_json::from_str(&v).unwrap_or(Value::String(v)))
    }

    pub fn probe(&mut self, path: &str) -> anyhow::Result<Value> {
        self.request("GET", path, None, true)
    }
    pub fn post(&mut self, path: &str, body: &Value) -> anyhow::Result<Value> {
        self.request("POST", path, Some(body), false)
    }

    pub fn post_raw(&mut self, path: &str, body: &Value) -> anyhow::Result<String> {
        self.request_raw("POST", path, Some(body), false)
    }

    fn request(&mut self, method: &str, path: &str, body: Option<&Value>, once: bool) -> anyhow::Result<Value> {
        let raw = self.request_raw(method, path, body, once)?;
        Ok(serde_json::from_str(&raw).unwrap_or(Value::String(raw)))
    }

    fn request_raw(&mut self, method: &str, path: &str, body: Option<&Value>, once: bool) -> anyhow::Result<String> {
        let url = format!("{}{}", self.config.base_url.trim_end_matches('/'), path);
        for attempt in 0..=if once { 0 } else { 3 } {
            self.limiter().wait();
            let mut request = match method {
                "GET" => self.http.get(&url),
                "POST" => self.http.post(&url),
                _ => anyhow::bail!("unsupported HTTP method"),
            }
            .header(
                "User-Agent",
                format!(
                    "mixlayer-cli/{} ({}; {})",
                    env!("CARGO_PKG_VERSION"),
                    std::env::consts::OS,
                    std::env::consts::ARCH
                ),
            )
            .header("Accept", "application/json");
            if !self.config.auth_header.is_empty() {
                request = request.header("Authorization", self.config.auth_header.clone());
            }
            for (key, value) in &self.config.headers {
                request = request.header(key.as_str(), value.as_str());
            }
            if let Some(body) = body {
                request = request.json(body);
            }
            let response = match request.send() {
                Ok(r) => r,
                Err(e) => {
                    let masked = mask_credential(&error_chain(&e), &self.config.auth_header);
                    if method == "GET" && attempt < 3 && !once {
                        continue;
                    }
                    anyhow::bail!("{method} {path}: {masked}");
                }
            };
            let status = response.status().as_u16();
            let retry_after = response
                .headers()
                .get("Retry-After")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_owned();
            let bytes = response.bytes()?;
            let text = String::from_utf8_lossy(&bytes).into_owned();
            if status < 400 {
                self.limiter().success();
                if method != "GET" {
                    let _ = fs::remove_dir_all(&self.cache_dir);
                }
                return Ok(text);
            }
            let trimmed = mask_credential(&truncate_body(&text), &self.config.auth_header);
            let error = ApiError {
                method: method.into(),
                path: path.into(),
                status,
                body: trimmed,
                retry_after: retry_after.clone(),
            };
            if status == 429 && attempt < 3 && !once {
                self.limiter().throttle();
                let wait = retry_after_wait(&retry_after);
                eprintln!(
                    "rate limited, waiting {wait:?} (attempt {}/{}, rate adjusted to {:.1} req/s)",
                    attempt + 1,
                    3,
                    self.limiter().rate
                );
                thread::sleep(wait);
                continue;
            }
            if status >= 500 && method == "GET" && attempt < 3 && !once {
                let wait = Duration::from_secs(1 << attempt);
                eprintln!(
                    "server error {status}, retrying in {wait:?} (attempt {}/{})",
                    attempt + 1,
                    3
                );
                thread::sleep(wait);
                continue;
            }
            return Err(error.into());
        }
        anyhow::bail!("request retries exhausted")
    }
}

fn mask_credential(text: &str, auth: &str) -> String {
    let auth = auth.trim();
    if auth.is_empty() {
        return text.to_owned();
    }
    let masked = text.replace(auth, "[REDACTED]");
    match auth.split_once(' ') {
        Some((_, secret)) if !secret.is_empty() => masked.replace(secret, "[REDACTED]"),
        _ => masked,
    }
}

fn truncate_body(text: &str) -> String {
    if text.len() <= 4096 {
        return text.to_owned();
    }
    let mut n = 4096;
    while !text.is_char_boundary(n) {
        n -= 1;
    }
    format!("{}...", &text[..n])
}

pub(crate) fn retry_after_wait(raw: &str) -> Duration {
    let raw = raw.trim();
    if let Ok(seconds) = raw.parse::<i64>() {
        if seconds <= 0 {
            return Duration::from_secs(5);
        }
        if seconds > 1_000_000_000 {
            let epoch = if seconds > 1_000_000_000_000 {
                seconds / 1000
            } else {
                seconds
            };
            let now = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64;
            let remaining = epoch - now;
            return Duration::from_secs(if remaining > 0 { remaining.min(60) as u64 } else { 60 });
        }
        return Duration::from_secs(seconds.clamp(0, 60) as u64);
    }
    if let Ok(date) = chrono::DateTime::parse_from_rfc2822(raw) {
        let remaining = date.timestamp() - chrono::Utc::now().timestamp();
        return Duration::from_secs(if remaining > 0 { remaining.min(60) as u64 } else { 5 });
    }
    Duration::from_secs(5)
}

/// An error and its causes, joined like Go's wrapped errors ("error sending
/// request for url (...): client error (Connect): connection refused"), so
/// network failures say why.
pub fn error_chain(e: &(dyn std::error::Error + 'static)) -> String {
    let mut out = e.to_string();
    let mut cur = e.source();
    while let Some(src) = cur {
        let text = src.to_string();
        if !out.contains(&text) {
            out.push_str(": ");
            out.push_str(&text);
        }
        cur = src.source();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_and_retry_wait_match_go() {
        assert_eq!(parse_duration("1m0s").unwrap(), Duration::from_secs(60));
        assert_eq!(parse_duration("40ms").unwrap(), Duration::from_millis(40));
        assert_eq!(retry_after_wait("0"), Duration::from_secs(5));
        assert_eq!(retry_after_wait("999"), Duration::from_secs(60));
    }

    #[test]
    fn truncation_keeps_valid_utf8() {
        let input = format!("{}€{}", "a".repeat(4094), "b".repeat(100));
        assert_eq!(truncate_body(&input), format!("{}...", "a".repeat(4094)));
    }

    #[test]
    fn cache_is_scoped_to_origin_and_auth_without_storing_token() {
        let base = Config {
            path: PathBuf::from("/tmp/config.toml"),
            credentials_path: PathBuf::from("/tmp/credentials.toml"),
            base_url: "https://one.example/v1".into(),
            auth_header: "Bearer mlk-dummy-one".into(),
            auth_source: "env:MIXLAYER_API_KEY".into(),
            credential_source: "env:MIXLAYER_API_KEY".into(),
            headers: Vec::new(),
            legacy_path: None,
        };
        let a = Client::new(base.clone(), Duration::from_secs(1), 0.0, false).unwrap();
        let mut other_auth = base.clone();
        other_auth.auth_header = "Bearer mlk-dummy-two".into();
        let b = Client::new(other_auth, Duration::from_secs(1), 0.0, false).unwrap();
        let mut other_origin = base;
        other_origin.base_url = "https://two.example/v1".into();
        let c = Client::new(other_origin, Duration::from_secs(1), 0.0, false).unwrap();
        let key = a.cache_key("/models");
        assert_ne!(key, b.cache_key("/models"));
        assert_ne!(key, c.cache_key("/models"));
        assert!(!key.contains("mlk-dummy"));
    }

    #[test]
    fn error_masking_never_inserts_text_when_auth_missing() {
        assert_eq!(mask_credential("server error", ""), "server error");
        assert_eq!(
            mask_credential("Bearer mlk-dummy and mlk-dummy", "Bearer mlk-dummy"),
            "[REDACTED] and [REDACTED]"
        );
    }

    #[test]
    fn limiter_ramps_and_halves_like_go() {
        let mut limiter = Limiter::new(10.0);
        for _ in 0..10 {
            limiter.success();
        }
        assert_eq!(limiter.rate, 12.5);
        limiter.throttle();
        assert_eq!(limiter.rate, 6.25);
    }
}
