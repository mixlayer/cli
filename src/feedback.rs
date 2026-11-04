// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

//! Opt-in feedback loop: when `mxl` fails or a user runs `mxl feedback`, build a
//! scrubbed, prefilled GitHub issue link for mixlayer/cli that the user submits
//! themselves. The CLI holds no GitHub token and never files anything on its own.
//!
//! What a report may contain is an allowlist: the command path, the exit code
//! and its kind, the CLI version, OS and arch, a repeat count, and (for
//! `mxl feedback`) the user's own note after redaction. Prompts, model output,
//! error messages, API keys, environment values and file contents are never read.

use std::collections::BTreeMap;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use regex::Regex;
use sha2::{Digest, Sha256};

/// Reports go to the public CLI repo.
pub const REPO: &str = "mixlayer/cli";
/// Marks every issue mxl generates, so maintainers can filter them.
pub const TITLE_PREFIX: &str = "[mxl-feedback]";
/// Env var that sets the mode and overrides config. `never` disables everything.
pub const ENV_MODE: &str = "MXL_FEEDBACK";
pub const MAX_NOTE: usize = 500;
/// At most one offer per fingerprint in this window.
pub const PER_FINGERPRINT_SECS: i64 = 24 * 3600;
/// At most this many offers per local day.
pub const PER_DAY: u32 = 5;
/// Failures of the same fingerprint inside this window count as a retry loop.
pub const RETRY_WINDOW_SECS: i64 = 10 * 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Ask,
    Always,
    Never,
}

impl Mode {
    pub fn parse(s: &str) -> Option<Mode> {
        match s.trim().to_ascii_lowercase().as_str() {
            "ask" => Some(Mode::Ask),
            "always" => Some(Mode::Always),
            "never" | "off" | "0" | "false" => Some(Mode::Never),
            _ => None,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Ask => "ask",
            Mode::Always => "always",
            Mode::Never => "never",
        }
    }
}

/// Env beats config; anything unrecognized falls back to the default, `ask`.
pub fn resolve_mode(env: Option<&str>, config: Option<&str>) -> Mode {
    env.and_then(Mode::parse)
        .or_else(|| config.and_then(Mode::parse))
        .unwrap_or(Mode::Ask)
}

/// `feedback.mode` from the config file in use (`--config`, `MIXLAYER_CONFIG`
/// or the default `config.toml`), e.g. `[feedback] mode = "never"`.
pub fn config_mode(explicit: Option<&str>) -> Option<String> {
    let path = crate::api::config::selected_path(explicit).ok()?;
    let text = std::fs::read_to_string(path).ok()?;
    let v: toml::Value = toml::from_str(&text).ok()?;
    v.get("feedback")?.get("mode")?.as_str().map(str::to_string)
}

pub fn effective_mode(config: Option<&str>) -> Mode {
    let env = std::env::var(ENV_MODE).ok();
    resolve_mode(env.as_deref(), config_mode(config).as_deref())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Context {
    pub interactive: bool,
    pub agent: bool,
    pub no_input: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Offer {
    /// Say nothing.
    Skip,
    /// Ask on stderr before showing a link.
    Prompt,
    /// Show the link without asking (the user still submits it).
    Show,
}

/// What to do after a failure. Never prompts without a TTY or in agent mode.
pub fn decide_after_error(mode: Mode, ctx: Context) -> Offer {
    if mode == Mode::Never || ctx.agent || !ctx.interactive {
        return Offer::Skip;
    }
    match mode {
        Mode::Always => Offer::Show,
        _ if ctx.no_input => Offer::Skip,
        _ => Offer::Prompt,
    }
}

// ---------- redaction ----------

fn re(cell: &'static OnceLock<Regex>, pat: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pat).expect("valid regex"))
}

/// Scrub anything identifying or secret from free text.
pub fn scrub(text: &str, home: Option<&Path>) -> String {
    static BEARER: OnceLock<Regex> = OnceLock::new();
    static KEYISH: OnceLock<Regex> = OnceLock::new();
    static ASSIGN: OnceLock<Regex> = OnceLock::new();
    static LONG: OnceLock<Regex> = OnceLock::new();
    static EMAIL: OnceLock<Regex> = OnceLock::new();
    static URL: OnceLock<Regex> = OnceLock::new();
    static IP: OnceLock<Regex> = OnceLock::new();
    static UNIX_PATH: OnceLock<Regex> = OnceLock::new();
    static WIN_PATH: OnceLock<Regex> = OnceLock::new();
    static HOST: OnceLock<Regex> = OnceLock::new();

    let mut s = text.to_string();
    if let Some(h) = home.and_then(|h| h.to_str()).filter(|h| h.len() > 1) {
        s = s.replace(h, "~");
    }
    s = re(&BEARER, r"(?i)\b(bearer|token|basic)\s+[A-Za-z0-9._~+/=-]{8,}")
        .replace_all(&s, "$1 [secret]")
        .into_owned();
    s = re(
        &KEYISH,
        r"\b(?:sk|pk|rk|mxl|ml|mix|gh[pousr]|github_pat|xox[abpr]|AKIA)[-_][A-Za-z0-9_-]{8,}\b",
    )
    .replace_all(&s, "[secret]")
    .into_owned();
    s = re(
        &ASSIGN,
        r"(?i)\b([A-Z0-9_]*(?:KEY|TOKEN|SECRET|PASSWORD|PASS)[A-Z0-9_]*)\s*[=:]\s*\S+",
    )
    .replace_all(&s, "$1=[secret]")
    .into_owned();
    s = re(&EMAIL, r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}")
        .replace_all(&s, "[email]")
        .into_owned();
    // Allowed URLs are parked behind placeholders so later passes can't touch them.
    // Only Mixlayer's own repos on GitHub and mixlayer.com pages are kept, and
    // never their query string or fragment (a private repo path or a
    // `?token=` value must not reach a public issue).
    let mut kept: Vec<String> = Vec::new();
    s = re(&URL, r"(?i)\bhttps?://([^/\s?#]+)([^\s?#]*)[^\s]*")
        .replace_all(&s, |c: &regex::Captures| {
            let host = c[1].to_ascii_lowercase();
            let path = &c[2];
            let ours = (host == "github.com" && path.to_ascii_lowercase().starts_with("/mixlayer/"))
                || host == "mixlayer.com"
                || host.ends_with(".mixlayer.com");
            if ours {
                kept.push(format!("https://{host}{path}"));
                format!("\u{1}{}\u{1}", kept.len() - 1)
            } else {
                "[url]".to_string()
            }
        })
        .into_owned();
    s = re(
        &IP,
        r"\b(?:\d{1,3}\.){3}\d{1,3}\b|\b(?:[0-9a-fA-F]{1,4}:){4,7}[0-9a-fA-F]{1,4}\b",
    )
    .replace_all(&s, "[ip]")
    .into_owned();
    // A path keeps going across a space while the next word is itself a path
    // segment (`C:\Work\Jane Doe\x.txt`, `/tmp/Jane Doe/x.txt`), so no piece of
    // a spaced directory name is left behind.
    s = re(&WIN_PATH, r"(?i)\b[A-Z]:\\[^\s]*(?:[ \t]+[^\s]*\\[^\s]*)*")
        .replace_all(&s, "[path]")
        .into_owned();
    s = re(
        &UNIX_PATH,
        r"(?:~|\.{1,2})?/[A-Za-z0-9._@%+-]+(?:/[A-Za-z0-9._@%+-]*)*(?:[ \t]+[A-Za-z0-9._@%+-]+/[A-Za-z0-9._@%+-]*(?:/[A-Za-z0-9._@%+-]*)*)*",
    )
        .replace_all(&s, |c: &regex::Captures| {
            // Keep the GitHub/Mixlayer URLs restored above intact.
            if c[0].starts_with("//") {
                c[0].to_string()
            } else {
                "[path]".to_string()
            }
        })
        .into_owned();
    s = re(
        &HOST,
        r"(?i)\b(?:[a-z0-9-]+\.)+(?:local|lan|internal|corp|home|localdomain|com|net|org|io|dev|ai|co|app|cloud)\b",
    )
    .replace_all(&s, |c: &regex::Captures| {
        let h = c[0].to_ascii_lowercase();
        if h == "github.com" || h == "mixlayer.com" || h.ends_with(".mixlayer.com") {
            c[0].to_string()
        } else {
            "[host]".to_string()
        }
    })
    .into_owned();
    // Long opaque runs (hex, base64) that slipped past the patterns above.
    s = re(&LONG, r"\b[A-Za-z0-9+/_=-]{32,}\b")
        .replace_all(&s, "[secret]")
        .into_owned();
    for (i, u) in kept.iter().enumerate() {
        s = s.replace(&format!("\u{1}{i}\u{1}"), u);
    }
    let s = s.trim();
    if s.chars().count() > MAX_NOTE {
        // The ellipsis counts toward the cap.
        let cut: String = s.chars().take(MAX_NOTE - 1).collect();
        format!("{cut}…")
    } else {
        s.to_string()
    }
}

/// Command names only: keep leading words that are real `mxl` subcommands, so
/// neither a positional argument nor free text passed to `--command` can leak
/// into a report.
pub fn clean_command(path: &[&str]) -> String {
    let root = crate::cli::build();
    let mut cmd = &root;
    let mut out: Vec<String> = Vec::new();
    for p in path.iter().take(3) {
        match cmd.find_subcommand(p) {
            Some(sub) => {
                out.push(sub.get_name().to_string());
                cmd = sub;
            }
            None => break,
        }
    }
    out.join(" ")
}

// ---------- report ----------

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Bug,
    Confusing,
    Slow,
    Missing,
    /// Automatic: a command failed.
    Error,
    /// Automatic: the same failure repeated inside the retry window.
    Friction,
}

impl Kind {
    pub fn parse(s: &str) -> Option<Kind> {
        match s {
            "bug" => Some(Kind::Bug),
            "confusing" => Some(Kind::Confusing),
            "slow" => Some(Kind::Slow),
            "missing" => Some(Kind::Missing),
            _ => None,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Kind::Bug => "bug",
            Kind::Confusing => "confusing",
            Kind::Slow => "slow",
            Kind::Missing => "missing",
            Kind::Error => "error",
            Kind::Friction => "friction",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Report {
    pub kind: Kind,
    pub command: String,
    pub exit_code: Option<u8>,
    pub error_kind: Option<String>,
    pub version: String,
    pub os: String,
    pub arch: String,
    pub repeats: u32,
    /// Already scrubbed.
    pub note: Option<String>,
    pub automatic: bool,
}

impl Report {
    pub fn new(kind: Kind, command: String, version: &str) -> Report {
        Report {
            kind,
            command,
            exit_code: None,
            error_kind: None,
            version: version.to_string(),
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            repeats: 1,
            note: None,
            automatic: false,
        }
    }

    /// Stable across runs: command + error kind (or report kind) + version.
    /// Automatic reports leave the note out, so two people hitting the same
    /// failure match. `mxl feedback` reports (no error kind) include it, so two
    /// different notes about the same command never count as duplicates.
    pub fn fingerprint(&self) -> String {
        match (&self.error_kind, &self.note) {
            (Some(ek), _) => fingerprint(&self.command, ek, &self.version),
            (None, Some(n)) => fingerprint(
                &self.command,
                &format!("{}\u{1f}{n}", self.kind.as_str()),
                &self.version,
            ),
            (None, None) => fingerprint(&self.command, self.kind.as_str(), &self.version),
        }
    }

    pub fn title(&self) -> String {
        let cmd = if self.command.is_empty() {
            "mxl".to_string()
        } else {
            format!("mxl {}", self.command)
        };
        match (&self.error_kind, &self.note) {
            (Some(ek), _) if self.automatic => {
                format!("{TITLE_PREFIX} {cmd}: {ek} (exit {})", self.exit_code.unwrap_or(1))
            }
            (_, Some(n)) => {
                let short: String = n.chars().take(60).collect();
                format!("{TITLE_PREFIX} [{}] {cmd}: {short}", self.kind.as_str())
            }
            _ => format!("{TITLE_PREFIX} [{}] {cmd}", self.kind.as_str()),
        }
    }

    pub fn labels(&self) -> Vec<String> {
        let mut l = vec![
            "feedback".to_string(),
            "automated".to_string(),
            if self.automatic { "auto" } else { "user-report" }.to_string(),
            format!("kind:{}", self.kind.as_str()),
        ];
        if !self.command.is_empty() {
            l.push(format!("cmd:{}", self.command.replace(' ', "-")));
        }
        if let Some(c) = self.exit_code {
            l.push(format!("exit:{c}"));
        }
        l.push(format!("v{}", self.version));
        l.push(format!("os:{}-{}", self.os, self.arch));
        l
    }

    pub fn body(&self) -> String {
        let mut b = String::new();
        if let Some(n) = &self.note {
            b.push_str(n);
            b.push_str("\n\n");
        } else {
            b.push_str(
                "<!-- Optional: what were you trying to do? Please don't paste prompts, keys or private data. -->\n\n",
            );
        }
        b.push_str("| field | value |\n|---|---|\n");
        let mut row = |k: &str, v: &str| b.push_str(&format!("| {k} | `{v}` |\n"));
        row("command", format!("mxl {}", self.command).trim_end());
        row("kind", self.kind.as_str());
        if let Some(ek) = &self.error_kind {
            row("error kind", ek);
        }
        if let Some(c) = self.exit_code {
            row("exit code", &c.to_string());
        }
        if self.repeats > 1 {
            row("repeats (10 min)", &self.repeats.to_string());
        }
        row("version", &self.version);
        row("os/arch", &format!("{}/{}", self.os, self.arch));
        b.push_str(&format!(
            "\nAutomated report generated by `mxl` (label `automated`). No prompts, outputs, keys or environment values are included.\n\nfingerprint: `{}`\n",
            self.fingerprint()
        ));
        b
    }
}

pub fn fingerprint(command: &str, kind: &str, version: &str) -> String {
    let h = Sha256::digest(format!("{command}\u{1f}{kind}\u{1f}{version}").as_bytes());
    format!("mxl-{}", hex::encode(&h[..6]))
}

/// Prefilled `issues/new` link. The user reviews and submits it on GitHub.
pub fn issue_url(r: &Report) -> String {
    let mut u = url::Url::parse(&format!("https://github.com/{REPO}/issues/new")).expect("static url");
    u.query_pairs_mut()
        .append_pair("title", &r.title())
        .append_pair("body", &r.body())
        .append_pair("labels", &r.labels().join(","));
    u.to_string()
}

/// Public issue search for a fingerprint (no login needed to view).
pub fn search_url(fp: &str) -> String {
    let mut u = url::Url::parse(&format!("https://github.com/{REPO}/issues")).expect("static url");
    u.query_pairs_mut().append_pair("q", &format!("is:issue \"{fp}\""));
    u.to_string()
}

/// Look for an existing issue with this fingerprint through GitHub's public,
/// unauthenticated search API. Any failure (offline, rate limited) is `None`.
pub fn find_existing(fp: &str) -> Option<String> {
    let mut u = url::Url::parse("https://api.github.com/search/issues").ok()?;
    u.query_pairs_mut()
        .append_pair("q", &format!("repo:{REPO} is:issue \"{fp}\""))
        .append_pair("per_page", "1");
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(4))
        .user_agent(format!("mxl/{}", crate::cli::VERSION))
        .build()
        .ok()?;
    let v: serde_json::Value = client.get(u).send().ok()?.error_for_status().ok()?.json().ok()?;
    v["items"][0]["html_url"].as_str().map(str::to_string)
}

// ---------- local rate limits ----------

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Ledger {
    /// fingerprint -> unix seconds of the last offer shown
    pub offered: BTreeMap<String, i64>,
    /// fingerprint -> recent failure times (for retry detection)
    pub failures: BTreeMap<String, Vec<i64>>,
    /// local day (YYYY-MM-DD) and offers shown that day
    pub day: String,
    pub day_count: u32,
}

impl Ledger {
    pub fn load(path: &Path) -> Ledger {
        let Ok(text) = std::fs::read_to_string(path) else {
            return Ledger::default();
        };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
            return Ledger::default();
        };
        let map_i = |k: &str| -> BTreeMap<String, i64> {
            v[k].as_object()
                .map(|o| o.iter().filter_map(|(k, v)| Some((k.clone(), v.as_i64()?))).collect())
                .unwrap_or_default()
        };
        let failures = v["failures"]
            .as_object()
            .map(|o| {
                o.iter()
                    .map(|(k, v)| {
                        (
                            k.clone(),
                            v.as_array()
                                .map(|a| a.iter().filter_map(|x| x.as_i64()).collect())
                                .unwrap_or_default(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ledger {
            offered: map_i("offered"),
            failures,
            day: v["day"].as_str().unwrap_or_default().to_string(),
            day_count: v["day_count"].as_u64().unwrap_or(0) as u32,
        }
    }

    /// Write the ledger. An error means the offer could not be recorded, so the
    /// caller must not show it (otherwise the rate limits stop applying).
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let v = serde_json::json!({
            "offered": self.offered,
            "failures": self.failures,
            "day": self.day,
            "day_count": self.day_count,
        });
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        // Write a sibling temp file (created 0600) and rename it into place, so
        // a reader never sees a half-written ledger.
        let tmp = path.with_extension("json.tmp");
        let written = write_private(&tmp, v.to_string().as_bytes()).and_then(|()| std::fs::rename(&tmp, path));
        if written.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        written
    }

    /// Hold an exclusive lock on a sibling `.lock` file from load through
    /// save, so two failing commands can't both pass the rate-limit check.
    /// `None` (lock unavailable) means skip the offer rather than risk a double one.
    pub fn lock(path: &Path) -> Option<std::fs::File> {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let f = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path.with_extension("lock"))
            .ok()?;
        f.lock().ok()?;
        Some(f)
    }

    /// Record a failure and return how many happened inside the retry window.
    pub fn record_failure(&mut self, fp: &str, now: i64) -> u32 {
        let list = self.failures.entry(fp.to_string()).or_default();
        list.retain(|t| now - *t < RETRY_WINDOW_SECS);
        list.push(now);
        // Keep the file small.
        self.failures.retain(|_, v| !v.is_empty());
        self.failures.get(fp).map(|v| v.len() as u32).unwrap_or(1)
    }

    pub fn allowed(&self, fp: &str, now: i64, today: &str) -> bool {
        let fresh = self.offered.get(fp).is_none_or(|t| now - *t >= PER_FINGERPRINT_SECS);
        fresh && self.budget_left(today)
    }

    pub fn budget_left(&self, today: &str) -> bool {
        self.day != today || self.day_count < PER_DAY
    }

    /// The first repeat inside the retry window (`repeats == 2`) is a friction
    /// report and gets its own offer even though the first failure used up the
    /// fingerprint's 24-hour slot. Later repeats and everything else follow
    /// `allowed`. The daily budget always applies.
    pub fn should_offer(&self, fp: &str, repeats: u32, now: i64, today: &str) -> bool {
        if repeats == 2 {
            self.budget_left(today)
        } else {
            self.allowed(fp, now, today)
        }
    }

    pub fn mark_offered(&mut self, fp: &str, now: i64, today: &str) {
        self.offered.insert(fp.to_string(), now);
        if self.day != today {
            self.day = today.to_string();
            self.day_count = 0;
        }
        self.day_count += 1;
        self.offered.retain(|_, t| now - *t < PER_FINGERPRINT_SECS * 7);
    }
}

/// Create (mode 0600 on Unix, set in the same open) or overwrite `path`.
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(path)?;
    f.write_all(bytes)?;
    #[cfg(unix)]
    {
        // `mode` applies only on create; tighten a leftover file too.
        use std::os::unix::fs::PermissionsExt;
        let _ = f.set_permissions(std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

pub fn ledger_path() -> Option<PathBuf> {
    crate::paths::dir(crate::paths::Kind::State)
        .ok()
        .map(|d| d.join("feedback.json"))
}

fn now_and_today() -> (i64, String) {
    let now = chrono::Local::now();
    (now.timestamp(), now.format("%Y-%m-%d").to_string())
}

// ---------- user-facing flows ----------

fn ask_yes(question: &str) -> bool {
    eprint!("{question} [y/N] ");
    let _ = std::io::stderr().flush();
    let mut line = String::new();
    if std::io::stdin().read_line(&mut line).is_err() {
        return false;
    }
    matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

pub fn open_in_browser(url: &str) -> bool {
    // Same launcher as `mxl login`: on Windows it goes through rundll32, not
    // `cmd /C start`, so `&` in the issue URL is never read as a command separator.
    crate::login::open_browser(url)
}

pub fn exit_kind(code: u8) -> String {
    crate::exit::TABLE
        .iter()
        .find(|(c, _)| *c as u8 == code)
        .map(|(_, d)| d.split(" (").next().unwrap_or(d).replace(' ', "-"))
        .unwrap_or_else(|| "other-failure".to_string())
}

/// Called from `main` after a command fails. Prints only to stderr and never
/// changes the exit code. Does nothing unless a person is at a terminal.
/// `config` is the `--config` value, if any.
pub fn offer_after_error(path: &[&str], code: u8, g: &crate::Globals, config: Option<&str>) {
    if path.first() == Some(&"feedback") || code == 0 {
        return;
    }
    // Exit 6 (partial / PII found by shield) and 9 (gate matched) are expected
    // outcomes that scripts branch on, not friction.
    if code == crate::exit::Code::Partial as u8 || code == crate::exit::Code::GateMatched as u8 {
        return;
    }
    let ctx = Context {
        interactive: std::io::stdin().is_terminal() && std::io::stderr().is_terminal(),
        agent: g.agent || crate::agent_detected(),
        no_input: g.no_input,
    };
    let offer = decide_after_error(effective_mode(config), ctx);
    if offer == Offer::Skip {
        return;
    }
    let Some(lpath) = ledger_path() else { return };
    let Some(lock) = Ledger::lock(&lpath) else { return };
    let mut ledger = Ledger::load(&lpath);
    let (now, today) = now_and_today();
    let mut r = Report::new(Kind::Error, clean_command(path), crate::cli::VERSION);
    r.automatic = true;
    r.exit_code = Some(code);
    r.error_kind = Some(exit_kind(code));
    let fp = r.fingerprint();
    r.repeats = ledger.record_failure(&fp, now);
    if r.repeats >= 2 {
        r.kind = Kind::Friction;
    }
    if !ledger.should_offer(&fp, r.repeats, now, &today) {
        let _ = ledger.save(&lpath);
        return;
    }
    ledger.mark_offered(&fp, now, &today);
    // If the offer can't be recorded, don't show it: the limits would stop
    // applying and every failing command would offer again.
    if ledger.save(&lpath).is_err() {
        return;
    }
    drop(lock);
    let show = match offer {
        Offer::Prompt => ask_yes(&format!(
            "\nmxl {} failed. Open a prefilled GitHub issue? Nothing is sent unless you submit it.",
            r.command
        )),
        _ => true,
    };
    if !show {
        eprintln!("(Turn these off with `export {ENV_MODE}=never` or `[feedback] mode = \"never\"`.)");
        return;
    }
    present(&r, offer == Offer::Prompt);
}

fn present(r: &Report, try_open: bool) {
    let fp = r.fingerprint();
    if let Some(existing) = find_existing(&fp) {
        eprintln!("Someone already reported this: {existing}\nA 👍 or comment there helps it get fixed.");
        if try_open {
            let _ = open_in_browser(&existing);
        }
        return;
    }
    let url = issue_url(r);
    eprintln!("Review and submit on GitHub (anything you submit is public):\n{url}");
    if try_open {
        let _ = open_in_browser(&url);
    }
}

/// `mxl feedback [message...] [--kind K] [--command C] [--open] [--dry-run]`
pub fn run_command(m: &clap::ArgMatches, g: &crate::Globals) -> anyhow::Result<()> {
    let config = m.try_get_one::<String>("config").ok().flatten().map(String::as_str);
    if effective_mode(config) == Mode::Never {
        return Err(crate::exit::err(
            crate::exit::Code::Config,
            format!("feedback is turned off ({ENV_MODE}=never or feedback.mode = \"never\")"),
        ));
    }
    let kind_s = m.get_one::<String>("kind").map(String::as_str).unwrap_or("bug");
    let kind = Kind::parse(kind_s).ok_or_else(|| {
        crate::exit::usage(format!(
            "invalid --kind {kind_s:?}: use bug, confusing, slow or missing"
        ))
    })?;
    let cmd_raw = m.get_one::<String>("command").cloned().unwrap_or_default();
    let words: Vec<&str> = cmd_raw.split_whitespace().filter(|w| *w != "mxl").collect();
    let note_raw = m
        .get_many::<String>("message")
        .map(|v| v.cloned().collect::<Vec<_>>().join(" "))
        .unwrap_or_default();
    let home = crate::paths::user_home();
    let mut r = Report::new(kind, clean_command(&words), crate::cli::VERSION);
    let note = scrub(&note_raw, home.as_deref());
    r.note = (!note.is_empty()).then_some(note);
    let url = issue_url(&r);
    let fp = r.fingerprint();
    if g.dry_run || g.json || g.agent {
        let v = serde_json::json!({
            "dry_run": g.dry_run,
            "repo": REPO,
            "title": r.title(),
            "body": r.body(),
            "labels": r.labels(),
            "fingerprint": fp,
            "search_url": search_url(&fp),
            "issue_url": url,
        });
        if g.json || g.agent {
            return crate::output::print_json(&v, g);
        }
        println!("Would open this issue on github.com/{REPO} (nothing is sent):\n");
        println!(
            "title:  {}\nlabels: {}\n\n{}\nlink: {}",
            r.title(),
            r.labels().join(", "),
            r.body(),
            url
        );
        return Ok(());
    }
    if let Some(existing) = find_existing(&fp) {
        println!("A matching issue already exists: {existing}\nAdd a 👍 or a comment there.");
        if m.get_flag("open") {
            let _ = open_in_browser(&existing);
        }
        return Ok(());
    }
    println!("{url}");
    if m.get_flag("open") {
        let _ = open_in_browser(&url);
    } else if std::io::stdout().is_terminal() {
        eprintln!(
            "Open that link to review and submit (submitted issues are public). Add --open to launch your browser."
        );
    }
    Ok(())
}

pub fn command() -> clap::Command {
    use clap::{Arg, ArgAction, Command};
    Command::new("feedback")
        .about("Open a prefilled GitHub issue about mxl (nothing is sent for you)")
        .long_about(
            "Builds a scrubbed, prefilled issue link for github.com/mixlayer/cli. You review and submit it \
             yourself; mxl holds no GitHub token. Paths, emails, hosts and key-like strings are removed from \
             your note, and prompts, outputs, keys and environment values are never included.\n\n\
             After a failed command in a terminal, mxl offers the same link. Control it with \
             feedback.mode (ask, always, never; default ask) in config.toml or MXL_FEEDBACK. It never prompts \
             without a terminal or in agent mode.",
        )
        .after_help(
            "Examples:\n  mxl feedback \"compare should accept --models from a file\" --kind missing --command compare\n  mxl feedback --dry-run \"ask hangs on a slow network\" --kind slow\n  MXL_FEEDBACK=never mxl ask \"...\"   # never offer",
        )
        .arg(Arg::new("message").num_args(0..).action(ArgAction::Append).help("what happened (scrubbed before use)"))
        .arg(Arg::new("kind").long("kind").value_name("KIND").help("bug, confusing, slow or missing (default bug)"))
        .arg(Arg::new("command").long("command").value_name("CMD").help("the mxl command it's about, e.g. \"models list\""))
        .arg(Arg::new("open").long("open").action(ArgAction::SetTrue).help("open the link in your browser"))
}

#[cfg(test)]
mod tests;
