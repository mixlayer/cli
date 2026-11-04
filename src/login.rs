// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

//! `mxl login`, `mxl logout` and `mxl billing` (balance, add-card, add-credit).
//!
//! Login works today without new backend work: it opens the console's API-keys
//! page, reads the key without echoing it (or from stdin with `--with-token`),
//! checks it with `GET /models`, and saves it where `auth set-token` does.
//! When the Platform API serves the device flow in [`endpoints`], `mxl login`
//! uses it first and falls back to the paste flow on 404/405/501.
//!
//! Billing uses endpoints that already exist: the billing account
//! (`?include=all`, for the prepaid balance) and `POST /refill` (buys credit with
//! the saved default card; needs an `api-admin` key and an explicit `--yes`).
//! Adding a card opens a hosted page: `card_setup_session` when the backend
//! has it, otherwise the console billing page.

use std::io::{IsTerminal, Read, Write};
use std::time::{Duration, Instant};

use clap::{Arg, ArgAction, ArgMatches, Command};
use serde_json::{Value, json};

use crate::Globals;
use crate::api::client::{ApiError, Client};
use crate::api::{config, console_url, platform_base};
use crate::exit::{self, Code};

/// Platform API paths. The `cli/device_authorizations*` and
/// `card_setup_session` paths are proposed in mixlayer/mixlayer and may 404 on
/// older deployments; every caller has a fallback.
pub mod endpoints {
    pub const DEVICE_START: &str = "/v1/cli/device_authorizations";
    pub const DEVICE_TOKEN: &str = "/v1/cli/device_authorizations/token";
    pub const ORGANIZATIONS: &str = "/v1/organizations";
    pub fn billing_account(id: &str) -> String {
        format!("/v1/billing_accounts/{}?include=all", super::seg(id))
    }
    pub fn refill(id: &str) -> String {
        format!("/v1/billing_accounts/{}/refill", super::seg(id))
    }
    pub fn card_setup_session(id: &str) -> String {
        format!("/v1/billing_accounts/{}/card_setup_session", super::seg(id))
    }
}

/// Console pages used by the browser fallbacks.
pub const KEYS_PAGE: &str = "/app/api-keys";
pub const BILLING_PAGE: &str = "/app/billing";

fn seg(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

// ---------- pure helpers (tested) ----------

/// Whether we may prompt or open a browser.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Interactivity {
    pub stdin_tty: bool,
    pub stderr_tty: bool,
    pub agent: bool,
    pub no_input: bool,
}

impl Interactivity {
    pub fn can_prompt(self) -> bool {
        self.stdin_tty && self.stderr_tty && !self.agent && !self.no_input
    }
}

/// How `mxl login` gets a key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoginPlan {
    /// Read the key from piped stdin.
    Stdin,
    /// Try the device flow, then fall back to pasting.
    DeviceThenPaste,
    /// Paste the key at a hidden prompt.
    Paste,
    /// Cannot log in without a terminal; tell the user how.
    Refuse,
}

pub fn plan_login(with_token: bool, paste: bool, i: Interactivity) -> LoginPlan {
    if with_token {
        return LoginPlan::Stdin;
    }
    if !i.can_prompt() {
        return LoginPlan::Refuse;
    }
    if paste {
        LoginPlan::Paste
    } else {
        LoginPlan::DeviceThenPaste
    }
}

/// Trim a pasted key and reject anything that can't be one.
pub fn clean_key(raw: &str) -> Result<String, &'static str> {
    let k = raw.trim().trim_start_matches("Bearer ").trim();
    if k.is_empty() {
        return Err("no API key entered");
    }
    if k.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("expected a single API key with no spaces");
    }
    if k.len() < 16 || k.len() > 512 {
        return Err("that doesn't look like a Mixlayer API key");
    }
    Ok(k.to_string())
}

/// The backend doesn't implement an endpoint yet.
pub fn is_unsupported(status: u16) -> bool {
    matches!(status, 404 | 405 | 501)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceStart {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: Option<String>,
    pub expires_in: u64,
    pub interval: u64,
}

pub fn parse_device_start(v: &Value) -> Option<DeviceStart> {
    let d = v.get("data").unwrap_or(v);
    let s = |k: &str| d.get(k).and_then(Value::as_str).map(str::to_string);
    let uri = s("verification_uri")?;
    // Only send people to Mixlayer's own console.
    let host = url::Url::parse(&uri).ok()?.host_str()?.to_string();
    if host != "console.mixlayer.com" && !host.ends_with(".mixlayer.com") {
        return None;
    }
    Some(DeviceStart {
        device_code: s("device_code")?,
        user_code: s("user_code")?,
        verification_uri: uri,
        verification_uri_complete: s("verification_uri_complete"),
        expires_in: d
            .get("expires_in")
            .and_then(Value::as_u64)
            .unwrap_or(600)
            .clamp(30, 3600),
        interval: d.get("interval").and_then(Value::as_u64).unwrap_or(5).clamp(1, 60),
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Poll {
    Pending,
    SlowDown,
    Approved { api_key: String },
    Denied,
    Expired,
}

pub fn parse_poll(v: &Value) -> Poll {
    let d = v.get("data").unwrap_or(v);
    match d.get("status").and_then(Value::as_str).unwrap_or("") {
        "approved" => match d.get("api_key").and_then(Value::as_str) {
            Some(k) if !k.is_empty() => Poll::Approved { api_key: k.to_string() },
            _ => Poll::Pending,
        },
        "slow_down" => Poll::SlowDown,
        "denied" => Poll::Denied,
        "expired" => Poll::Expired,
        _ => Poll::Pending,
    }
}

/// `--amount 25` or `25.50` dollars to cents, within the API's $1–$2,000.
pub fn amount_cents(raw: &str) -> Result<i64, String> {
    let r = raw.trim().trim_start_matches('$');
    let (whole, frac) = r.split_once('.').unwrap_or((r, ""));
    if whole.is_empty()
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || frac.len() > 2
        || !frac.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(format!("invalid --amount {raw:?}: use dollars, e.g. 25 or 25.50"));
    }
    let cents = whole.parse::<i64>().map_err(|_| format!("invalid --amount {raw:?}"))? * 100
        + format!("{frac:0<2}").parse::<i64>().unwrap_or(0);
    if !(100..=200_000).contains(&cents) {
        return Err("--amount must be between $1 and $2,000".into());
    }
    Ok(cents)
}

pub fn balance_usd(account: &Value) -> Option<f64> {
    let a = account.get("data").unwrap_or(account);
    if a.get("billing_mode").and_then(Value::as_str) != Some("prepaid") {
        return None;
    }
    a.get("credit_balance_cents").and_then(Value::as_f64).map(|c| c / 100.0)
}

// ---------- IO ----------

fn interactivity(g: &Globals) -> Interactivity {
    Interactivity {
        stdin_tty: std::io::stdin().is_terminal(),
        stderr_tty: std::io::stderr().is_terminal(),
        agent: g.agent || crate::agent_detected(),
        no_input: g.no_input,
    }
}

fn explicit_yes(m: &ArgMatches) -> bool {
    matches!(m.value_source("yes"), Some(clap::parser::ValueSource::CommandLine))
        && m.try_get_one::<bool>("yes").ok().flatten().copied() == Some(true)
}

fn bool_flag(m: &ArgMatches, name: &str) -> bool {
    m.try_get_one::<bool>(name).ok().flatten().copied().unwrap_or(false)
}

fn cfg_path(m: &ArgMatches) -> Option<&str> {
    m.try_get_one::<String>("config").ok().flatten().map(String::as_str)
}

pub fn open_browser(url: &str) -> bool {
    #[cfg(target_os = "macos")]
    let mut cmd = std::process::Command::new("open");
    #[cfg(windows)]
    let mut cmd = {
        let mut c = std::process::Command::new("rundll32");
        c.arg("url.dll,FileProtocolHandler");
        c
    };
    #[cfg(not(any(target_os = "macos", windows)))]
    let mut cmd = std::process::Command::new("xdg-open");
    cmd.arg(url)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .is_ok()
}

/// Read a line from the terminal without echoing it.
fn read_hidden(prompt: &str) -> anyhow::Result<String> {
    eprint!("{prompt}");
    std::io::stderr().flush()?;
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        let fd = std::io::stdin().as_raw_fd();
        // SAFETY: termios is plain data; tcgetattr fills it for a terminal fd.
        let mut old: libc::termios = unsafe { std::mem::zeroed() };
        if unsafe { libc::tcgetattr(fd, &mut old) } == 0 {
            let mut quiet = old;
            quiet.c_lflag &= !libc::ECHO;
            quiet.c_lflag |= libc::ECHONL;
            unsafe { libc::tcsetattr(fd, libc::TCSANOW, &quiet) };
            let mut line = String::new();
            let res = std::io::stdin().read_line(&mut line);
            unsafe { libc::tcsetattr(fd, libc::TCSANOW, &old) };
            res?;
            return Ok(line);
        }
    }
    #[cfg(not(unix))]
    {
        return Err(exit::usage(
            "hidden input isn't supported here; pipe the key instead: Get-Clipboard | mxl login --with-token",
        ));
    }
    #[allow(unreachable_code)]
    {
        let mut line = String::new();
        std::io::stdin().read_line(&mut line)?;
        Ok(line)
    }
}

fn client(m: &ArgMatches, key: Option<&str>) -> anyhow::Result<Client> {
    let mut cfg = config::load(cfg_path(m)).map_err(|e| exit::err(Code::Config, e.to_string()))?;
    if let Some(k) = key {
        cfg.auth_header = format!("Bearer {k}");
    }
    Client::new(cfg, Duration::from_secs(20), 0.0, true)
}

fn platform(m: &ArgMatches, key: Option<&str>) -> anyhow::Result<Client> {
    let mut c = client(m, key)?;
    c.config.base_url = platform_base()?;
    Ok(c)
}

fn status_of(e: &anyhow::Error) -> Option<u16> {
    e.downcast_ref::<ApiError>().map(|a| a.status)
}

/// Check a key against the inference API, the same probe `mxl doctor` uses.
fn verify_key(m: &ArgMatches, key: &str) -> anyhow::Result<()> {
    let mut c = client(m, Some(key))?;
    match c.probe("/models") {
        Ok(_) => Ok(()),
        Err(e) if matches!(status_of(&e), Some(401 | 403)) => Err(exit::err(
            Code::Auth,
            format!(
                "the API key was rejected; nothing was saved\nhint: create a new key at {}",
                console_url(KEYS_PAGE, "login")
            ),
        )),
        Err(e) => Err(exit::err(
            Code::Api,
            format!("could not verify the key; nothing was saved: {e}"),
        )),
    }
}

fn save_key(m: &ArgMatches, key: &str) -> anyhow::Result<std::path::PathBuf> {
    let cfg = config::load(cfg_path(m)).map_err(|e| exit::err(Code::Config, e.to_string()))?;
    cfg.save_token(key)
        .map_err(|e| exit::err(Code::Config, format!("saving token: {e}")))?;
    Ok(cfg.credentials_path)
}

/// Device flow. `Ok(None)` means the backend doesn't serve it.
fn device_login(m: &ArgMatches, open: bool) -> anyhow::Result<Option<String>> {
    let mut c = platform(m, Some("anonymous-device-flow"))?;
    c.config.auth_header.clear();
    let started = match c.post(
        endpoints::DEVICE_START,
        &json!({"client_name": format!("mxl {}", crate::cli::VERSION)}),
    ) {
        Ok(v) => v,
        Err(e) if status_of(&e).is_some_and(is_unsupported) => return Ok(None),
        Err(e) if status_of(&e).is_none() => return Ok(None),
        Err(e) => return Err(e),
    };
    let Some(d) = parse_device_start(&started) else {
        return Ok(None);
    };
    let url = d
        .verification_uri_complete
        .clone()
        .unwrap_or_else(|| d.verification_uri.clone());
    eprintln!("Confirm this code in your browser: {}\n  {url}", d.user_code);
    if open {
        let _ = open_browser(&url);
    }
    let deadline = Instant::now() + Duration::from_secs(d.expires_in);
    let mut interval = d.interval;
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_secs(interval));
        let v = match c.post(endpoints::DEVICE_TOKEN, &json!({"device_code": d.device_code})) {
            Ok(v) => v,
            Err(e) if matches!(status_of(&e), Some(429)) => {
                interval += 5;
                continue;
            }
            Err(e) => return Err(e),
        };
        match parse_poll(&v) {
            Poll::Pending => {}
            Poll::SlowDown => interval += 5,
            Poll::Approved { api_key } => return Ok(Some(api_key)),
            Poll::Denied => return Err(exit::err(Code::Auth, "login was denied in the browser")),
            Poll::Expired => break,
        }
    }
    Err(exit::err(Code::Auth, "the login code expired; run mxl login again"))
}

pub fn run_login(m: &ArgMatches, g: &Globals) -> anyhow::Result<()> {
    let plan = plan_login(bool_flag(m, "with-token"), bool_flag(m, "paste"), interactivity(g));
    let keys_url = console_url(KEYS_PAGE, "login");
    if g.dry_run {
        return crate::output::print_json(
            &json!({"dry_run": true, "plan": format!("{plan:?}"), "device_flow": endpoints::DEVICE_START,
                "keys_page": keys_url, "verify": "GET /models"}),
            g,
        );
    }
    let open = !bool_flag(m, "no-browser");
    let key = match plan {
        LoginPlan::Refuse => {
            return Err(exit::usage(format!(
                "mxl login needs a terminal; in scripts and agents pipe a key instead:\n  printf %s \"$KEY\" | mxl login --with-token\nCreate a key at {keys_url}"
            )));
        }
        LoginPlan::Stdin => {
            if std::io::stdin().is_terminal() {
                return Err(exit::usage(
                    "--with-token reads the key from stdin; pipe it in, e.g. pbpaste | mxl login --with-token",
                ));
            }
            let mut bytes = Vec::new();
            std::io::stdin().take(64 * 1024).read_to_end(&mut bytes)?;
            String::from_utf8(bytes).map_err(|_| exit::usage("API key must be UTF-8"))?
        }
        LoginPlan::DeviceThenPaste => match device_login(m, open)? {
            Some(k) => k,
            None => paste_prompt(&keys_url, open)?,
        },
        LoginPlan::Paste => paste_prompt(&keys_url, open)?,
    };
    let key = clean_key(&key).map_err(exit::usage)?;
    verify_key(m, &key)?;
    let path = save_key(m, &key)?;
    if g.json {
        crate::output::print_json(
            &json!({"logged_in": true, "verified": true, "credentials_path": path}),
            g,
        )
    } else {
        println!("Logged in. Key verified and saved to {}", path.display());
        println!("Check it any time with: mxl whoami");
        Ok(())
    }
}

fn paste_prompt(keys_url: &str, open: bool) -> anyhow::Result<String> {
    eprintln!("Create an API key in the Mixlayer console, then paste it here.\n  {keys_url}");
    if open {
        let _ = open_browser(keys_url);
    }
    read_hidden("API key (hidden): ")
}

pub fn run_logout(m: &ArgMatches, g: &Globals) -> anyhow::Result<()> {
    crate::api::commands::dispatch(&["auth", "logout"], m, g).expect("auth logout is dispatched")
}

/// `--billing-account`, else the billing account of `--org` or the only org.
fn billing_account(m: &ArgMatches, c: &mut Client) -> anyhow::Result<String> {
    if let Some(b) = m
        .try_get_one::<String>("billing-account")
        .ok()
        .flatten()
        .filter(|s| !s.is_empty())
    {
        return Ok(b.clone());
    }
    let orgs = c
        .get_no_cache(endpoints::ORGANIZATIONS)
        .map_err(|e| perm_error(e, "api-read"))?;
    let list = orgs.get("data").cloned().unwrap_or(orgs);
    let want = m.try_get_one::<String>("org").ok().flatten();
    let items: Vec<&Value> = list
        .as_array()
        .into_iter()
        .flatten()
        .filter(|o| want.is_none_or(|w| o["id"].as_str() == Some(w.as_str())))
        .collect();
    match items.as_slice() {
        [one] => one["billing_account_id"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .ok_or_else(|| exit::err(Code::NotFound, "this organization has no billing account")),
        [] => Err(exit::err(Code::NotFound, "no organization visible to this key")),
        _ => Err(exit::usage("this key sees several organizations; pass --org <org-id>")),
    }
}

fn perm_error(e: anyhow::Error, needs: &str) -> anyhow::Error {
    match status_of(&e) {
        Some(401) => exit::err(Code::Auth, format!("{e}\nhint: run mxl login")),
        Some(403) => exit::err(
            Code::Auth,
            format!(
                "{e}\nhint: this needs an API key with {needs}; edit it at {}",
                console_url(KEYS_PAGE, "billing")
            ),
        ),
        Some(402) => exit::err(Code::Billing, e.to_string()),
        Some(404) => exit::err(Code::NotFound, e.to_string()),
        Some(_) => exit::err(Code::Api, e.to_string()),
        None => e,
    }
}

fn fetch_balance(c: &mut Client, ba: &str) -> anyhow::Result<Option<f64>> {
    let v = c
        .get_no_cache(&endpoints::billing_account(ba))
        .map_err(|e| perm_error(e, "api-read"))?;
    Ok(balance_usd(&v))
}

pub fn run_balance(m: &ArgMatches, g: &Globals) -> anyhow::Result<()> {
    let mut c = platform(m, None)?;
    let ba = billing_account(m, &mut c)?;
    let bal = fetch_balance(&mut c, &ba)?;
    if g.json {
        return crate::output::print_json(&json!({"billing_account_id": ba, "prepaid_balance_usd": bal}), g);
    }
    match bal {
        Some(b) => println!("Prepaid balance: ${b:.2}"),
        None => println!("This account is not prepaid; see mxl spend for usage."),
    }
    Ok(())
}

fn wait_for_change(c: &mut Client, ba: &str, before: Option<f64>, limit: Duration) -> anyhow::Result<Option<f64>> {
    let deadline = Instant::now() + limit;
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_secs(5));
        let now = fetch_balance(c, ba)?;
        if now != before {
            return Ok(now);
        }
    }
    Ok(before)
}

/// `mxl billing add-card`: open a hosted page to add or change the card.
pub fn run_add_card(m: &ArgMatches, g: &Globals) -> anyhow::Result<()> {
    let fallback = console_url(BILLING_PAGE, "add-card");
    let mut c = platform(m, None)?;
    let ba = billing_account(m, &mut c)?;
    if g.dry_run {
        return crate::output::print_json(
            &json!({"dry_run": true, "method": "POST", "url": format!("{}{}", c.config.base_url, endpoints::card_setup_session(&ba)),
                "fallback_url": fallback}),
            g,
        );
    }
    let (url, source) = match c.post(&endpoints::card_setup_session(&ba), &json!({})) {
        Ok(v) => match v.get("data").unwrap_or(&v).get("url").and_then(Value::as_str) {
            Some(u) if u.starts_with("https://") => (u.to_string(), "card_setup_session"),
            _ => (fallback.clone(), "console"),
        },
        Err(e) if status_of(&e).is_some_and(|s| is_unsupported(s) || s == 403) => (fallback.clone(), "console"),
        Err(e) => return Err(perm_error(e, "api-admin")),
    };
    let i = interactivity(g);
    if g.json || !i.can_prompt() {
        return crate::output::print_json(&json!({"billing_account_id": ba, "url": url, "source": source}), g);
    }
    eprintln!("Add or update your card here (the link is private to you):\n  {url}");
    if !bool_flag(m, "no-browser") {
        let _ = open_browser(&url);
    }
    Ok(())
}

/// `mxl billing add-credit`: buy credit with the saved card (`--amount --yes`),
/// or open the console to do it and optionally wait for the balance to change.
pub fn run_add_credit(m: &ArgMatches, g: &Globals) -> anyhow::Result<()> {
    let amount = m.try_get_one::<String>("amount").ok().flatten().cloned();
    // Validate before touching the network so typos fail fast as usage errors.
    let cents = amount.as_deref().map(amount_cents).transpose().map_err(exit::usage)?;
    let mut c = platform(m, None)?;
    let ba = billing_account(m, &mut c)?;
    if let Some(cents) = cents {
        let body = json!({"amount_in_cents": cents});
        if g.dry_run {
            return crate::output::print_json(
                &json!({"dry_run": true, "method": "POST", "url": format!("{}{}", c.config.base_url, endpoints::refill(&ba)), "body": body}),
                g,
            );
        }
        if !explicit_yes(m) {
            return Err(exit::usage(format!(
                "this charges ${:.2} to the saved card; pass --yes to confirm (agent mode doesn't imply it)",
                cents as f64 / 100.0
            )));
        }
        let r = c
            .post(&endpoints::refill(&ba), &body)
            .map_err(|e| perm_error(e, "api-admin"))?;
        let r = r.get("data").cloned().unwrap_or(r);
        let bal = fetch_balance(&mut c, &ba).ok().flatten();
        if g.json {
            return crate::output::print_json(&json!({"refill": r, "prepaid_balance_usd": bal}), g);
        }
        println!(
            "Credit purchase {}: {}",
            r["id"].as_str().unwrap_or("created"),
            r["status"].as_str().unwrap_or("submitted")
        );
        if let Some(b) = bal {
            println!("Prepaid balance: ${b:.2}");
        }
        return Ok(());
    }
    let url = console_url(BILLING_PAGE, "add-credit");
    let i = interactivity(g);
    if g.dry_run || g.json || !i.can_prompt() {
        return crate::output::print_json(
            &json!({"billing_account_id": ba, "url": url,
            "hint": "or buy with the saved card: mxl billing add-credit --amount 25 --yes"}),
            g,
        );
    }
    let before = fetch_balance(&mut c, &ba)?;
    eprintln!("Add credit in the console:\n  {url}");
    if !bool_flag(m, "no-browser") {
        let _ = open_browser(&url);
    }
    if bool_flag(m, "wait") {
        eprintln!("Waiting up to 10 minutes for the balance to change (Ctrl-C to stop)…");
        let after = wait_for_change(&mut c, &ba, before, Duration::from_secs(600))?;
        match (before, after) {
            (b, a) if a != b => println!("Prepaid balance: ${:.2}", a.unwrap_or(0.0)),
            _ => println!("No change yet. Check later with: mxl billing balance"),
        }
    }
    Ok(())
}

// ---------- clap ----------

fn org_args(c: Command) -> Command {
    c.arg(
        Arg::new("org")
            .long("org")
            .value_name("ORG_ID")
            .help("organization (default: the only one the key sees)"),
    )
    .arg(
        Arg::new("billing-account")
            .long("billing-account")
            .value_name("ID")
            .help("billing account ID"),
    )
}

pub fn login_command() -> Command {
    Command::new("login")
        .about("Log in: get an API key from the console, verify it and save it")
        .long_about(
            "Opens the console's API-keys page, reads the key you paste without echoing it, checks it with \
             GET /models and saves it to the credentials file (same place as auth set-token). When the \
             Platform API supports the CLI device flow, mxl login uses it instead: confirm a code in your \
             browser and the key arrives on its own. In scripts and agents use --with-token with the key on stdin.",
        )
        .after_help("Examples:\n  mxl login\n  pbpaste | mxl login --with-token\n  mxl login --paste --no-browser")
        .arg(
            Arg::new("with-token")
                .long("with-token")
                .action(ArgAction::SetTrue)
                .help("read the API key from stdin"),
        )
        .arg(
            Arg::new("paste")
                .long("paste")
                .action(ArgAction::SetTrue)
                .help("skip the device flow and paste a key"),
        )
        .arg(
            Arg::new("no-browser")
                .long("no-browser")
                .action(ArgAction::SetTrue)
                .help("print links instead of opening them"),
        )
}

pub fn logout_command() -> Command {
    Command::new("logout").about("Clear stored credentials (same as auth logout)")
}

pub fn billing_command() -> Command {
    Command::new("billing")
        .about("Prepaid balance, add a card, add credit")
        .subcommand(org_args(Command::new("balance").about("Show the prepaid credit balance")))
        .subcommand(
            org_args(Command::new("add-card").about("Open a hosted page to add or update your card"))
                .arg(Arg::new("no-browser").long("no-browser").action(ArgAction::SetTrue).help("print the link only")),
        )
        .subcommand(
            org_args(Command::new("add-credit").about("Buy prepaid credit (saved card with --amount, or in the console)"))
                .long_about(
                    "With --amount and an explicit --yes, buys credit with the account's saved default card \
                     (POST /refill; needs an api-admin key). Without --amount, opens the console billing page; \
                     --wait polls the balance until it changes.",
                )
                .after_help("Examples:\n  mxl billing add-credit\n  mxl billing add-credit --wait\n  mxl billing add-credit --amount 25 --yes")
                .arg(Arg::new("amount").long("amount").value_name("USD").help("dollars to buy, $1 to $2,000"))
                .arg(Arg::new("wait").long("wait").action(ArgAction::SetTrue).help("wait for the balance to change"))
                .arg(Arg::new("no-browser").long("no-browser").action(ArgAction::SetTrue).help("print the link only")),
        )
}

pub fn dispatch(path: &[&str], m: &ArgMatches, g: &Globals) -> Option<anyhow::Result<()>> {
    Some(match path {
        ["login"] => run_login(m, g),
        ["logout"] => run_logout(m, g),
        ["billing"] | ["billing", "balance"] => run_balance(m, g),
        ["billing", "add-card"] => run_add_card(m, g),
        ["billing", "add-credit"] => run_add_credit(m, g),
        _ => return None,
    })
}

#[cfg(test)]
mod tests;
