// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

//! `keys`: manage organization API keys through the Platform API
//! (`/v1/organizations/{org_id}/api_keys`), the same endpoints the console's
//! API keys page uses. Secrets are printed once, on stdout only, and never
//! written to the ledger or the HTTP cache.

use std::io::{BufRead, IsTerminal, Write};
use std::process::Command;

use clap::ArgMatches;
use serde_json::{Map, Value, json};

use super::{arg, bool_arg, classified, new_client, print, words};
use crate::Globals;
use crate::api::client::{ApiError, Client};
use crate::api::{config, console_url, platform_base};
use crate::exit::{self, Code};

/// Permission scopes the Platform API accepts, in increasing order of power.
pub const PERMISSIONS: &[&str] = &["inference", "api-read", "api-write", "api-admin"];

const ORG_PLACEHOLDER: &str = "<org-id>";

pub fn list(m: &ArgMatches, g: &Globals) -> anyhow::Result<()> {
    let base = platform_base()?;
    if g.dry_run {
        let org = arg(m, "org").filter(|s| !s.is_empty()).unwrap_or(ORG_PLACEHOLDER);
        return print(&dry_run("GET", &base, &keys_path(org), None), g);
    }
    let mut client = platform_client(m, &base)?;
    let org = org_id(m, g, &mut client)?;
    let raw = client
        .get_no_cache(&keys_path(&org))
        .map_err(|e| platform_error(e, g, "api-read"))?;
    let keys = data(raw);
    crate::output::print_endpoint(&serde_json::to_string(&keys)?, g, 0)
}

pub fn create(m: &ArgMatches, g: &Globals) -> anyhow::Result<()> {
    let permissions = permissions(m)?;
    let mut body = Map::new();
    if let Some(name) = arg(m, "name") {
        let name = name.trim();
        if name.chars().count() > 200 {
            return Err(exit::usage("--name is limited to 200 characters"));
        }
        body.insert("name".into(), name.into());
    }
    body.insert("permissions".into(), json!(permissions));
    if let Some(at) = arg(m, "expires-at").filter(|s| !s.is_empty()) {
        body.insert("expires_at".into(), at.into());
    }
    let body = Value::Object(body);
    let base = platform_base()?;
    if g.dry_run {
        let org = arg(m, "org").filter(|s| !s.is_empty()).unwrap_or(ORG_PLACEHOLDER);
        return print(&dry_run("POST", &base, &keys_path(org), Some(&body)), g);
    }
    let mut client = platform_client(m, &base)?;
    let org = org_id(m, g, &mut client)?;
    let created = client
        .post(&keys_path(&org), &body)
        .map_err(|e| platform_error(e, g, "api-admin"))?;
    deliver_secret(m, g, data(created), "Created")
}

pub fn rotate(m: &ArgMatches, g: &Globals) -> anyhow::Result<()> {
    let id = key_id(m, "keys rotate")?;
    let base = platform_base()?;
    if g.dry_run {
        let org = arg(m, "org").filter(|s| !s.is_empty()).unwrap_or(ORG_PLACEHOLDER);
        return print(
            &dry_run("POST", &base, &format!("{}/rotate", key_path(org, &id)), None),
            g,
        );
    }
    confirm(
        m,
        g,
        &format!("Rotate API key {id}? Anything using its current secret will stop working."),
    )?;
    let mut client = platform_client(m, &base)?;
    let org = org_id(m, g, &mut client)?;
    let rotated = client
        .post(&format!("{}/rotate", key_path(&org, &id)), &json!({}))
        .map_err(|e| platform_error(e, g, "api-admin"))?;
    deliver_secret(m, g, data(rotated), "Rotated")
}

pub fn delete(m: &ArgMatches, g: &Globals) -> anyhow::Result<()> {
    let id = key_id(m, "keys delete")?;
    let base = platform_base()?;
    if g.dry_run {
        let org = arg(m, "org").filter(|s| !s.is_empty()).unwrap_or(ORG_PLACEHOLDER);
        return print(&dry_run("DELETE", &base, &key_path(org, &id), None), g);
    }
    confirm(
        m,
        g,
        &format!("Delete API key {id}? Anything using it will stop working. This cannot be undone."),
    )?;
    let mut client = platform_client(m, &base)?;
    let org = org_id(m, g, &mut client)?;
    client
        .delete(&key_path(&org, &id))
        .map_err(|e| platform_error(e, g, "api-admin"))?;
    if g.json {
        print(&json!({"deleted": true, "id": id, "organization_id": org}), g)
    } else {
        println!("Deleted API key {id}");
        Ok(())
    }
}

pub fn mint(m: &ArgMatches, g: &Globals) -> anyhow::Result<()> {
    let command = words(m, "keys mint");
    if command.is_empty() {
        return Err(exit::usage("a command is required after --"));
    }
    let ttl = crate::api::client::parse_duration(arg(m, "ttl").unwrap_or("30m"))
        .map_err(|e| exit::usage(format!("invalid --ttl: {e}")))?;
    if ttl.is_zero() || ttl > std::time::Duration::from_secs(24 * 3600) {
        return Err(exit::usage("--ttl must be greater than 0 and no more than 24h"));
    }
    let env_name = arg(m, "key-env").unwrap_or("MIXLAYER_API_KEY");
    if !env_name
        .bytes()
        .enumerate()
        .all(|(i, b)| b.is_ascii_alphabetic() || b == b'_' || (i > 0 && b.is_ascii_digit()))
        || env_name.is_empty()
    {
        return Err(exit::usage("--key-env must be an environment variable name"));
    }
    let permissions = permissions(m)?;
    let expires =
        chrono::Utc::now() + chrono::Duration::from_std(ttl).map_err(|e| exit::usage(format!("invalid --ttl: {e}")))?;
    let expires = expires.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let name = arg(m, "name").map(str::to_owned).unwrap_or_else(|| {
        let first = std::path::Path::new(&command[0])
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(&command[0]);
        let suffix = format!("-{}", chrono::Utc::now().format("%Y%m%dT%H%M%SZ"));
        let first: String = first.chars().take(200 - "mint-".len() - suffix.len()).collect();
        format!("mint-{first}{suffix}")
    });
    if name.chars().count() > 200 {
        return Err(exit::usage("--name is limited to 200 characters"));
    }
    let body = json!({"name": name, "permissions": permissions, "expires_at": expires});
    let base = platform_base()?;
    if g.dry_run {
        let org = arg(m, "org").filter(|s| !s.is_empty()).unwrap_or(ORG_PLACEHOLDER);
        return print(
            &json!({"dry_run": true, "create": dry_run("POST", &base, &keys_path(org), Some(&body)),
            "delete": dry_run("DELETE", &base, &key_path(org, "<minted-key-id>"), None), "command": command, "key_env": env_name}),
            g,
        );
    }
    let mut client = platform_client(m, &base)?;
    let org = org_id(m, g, &mut client)?;
    // Keep terminal signals from ending this process once the POST can create
    // a key. The child restores their defaults and gets the same signal.
    let _signals = MintSignals::ignore();
    let created = data(
        client
            .post(&keys_path(&org), &body)
            .map_err(|e| platform_error(e, g, "api-admin"))?,
    );
    let id = created
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| exit::err(Code::Api, "mint response missing key ID"))?
        .to_owned();
    let secret = match created.get("key").and_then(Value::as_str) {
        Some(secret) => secret,
        None => {
            let _ = client.delete(&key_path(&org, &id));
            return Err(exit::err(Code::Api, "mint response missing secret"));
        }
    };
    if !g.quiet {
        eprintln!(
            "Minted API key {id} ({}), expires {expires}; running: {}",
            permissions.join(", "),
            command[0]
        );
    }
    let mut child = Command::new(&command[0]);
    child.args(&command[1..]).env(env_name, secret);
    #[cfg(unix)]
    unsafe {
        use std::os::unix::process::CommandExt;
        child.pre_exec(|| {
            libc::signal(libc::SIGINT, libc::SIG_DFL);
            libc::signal(libc::SIGTERM, libc::SIG_DFL);
            Ok(())
        });
    }
    let result = if MintSignals::terminated_before_start() {
        Err(std::io::Error::new(
            std::io::ErrorKind::Interrupted,
            "terminated before the command started",
        ))
    } else {
        child.spawn().and_then(|mut running| {
            MintSignals::track(&running);
            running.wait()
        })
    };
    let deletion = client.delete(&key_path(&org, &id));
    if deletion.is_ok() {
        if !g.quiet {
            eprintln!("Deleted API key {id}");
        }
    } else {
        eprintln!("Could not delete API key {id} (expires {expires}); retry: mixlayer keys delete {id} --yes");
    }
    let status = result.map_err(|e| exit::err(Code::Other, format!("running {}: {e}", command[0])))?;
    let child_code = status.code().unwrap_or_else(|| signal_exit_code(&status));
    if child_code != 0 {
        return Err(exit::ChildExit(child_code.clamp(1, 255) as u8).into());
    }
    deletion.map_err(|e| exit::err(Code::Api, format!("deleting minted API key {id}: {e}")))?;
    Ok(())
}

/// Shell convention for a child killed by a signal: 128 + the signal number.
#[cfg(unix)]
fn signal_exit_code(status: &std::process::ExitStatus) -> i32 {
    use std::os::unix::process::ExitStatusExt;
    128 + status.signal().unwrap_or(1)
}

/// Windows always reports an exit code; this is only a fallback.
#[cfg(not(unix))]
fn signal_exit_code(_: &std::process::ExitStatus) -> i32 {
    1
}

#[cfg(unix)]
struct MintSignals(libc::sighandler_t, libc::sighandler_t);

/// PID of the running `keys mint` child, for the SIGTERM handler (0 = none yet).
#[cfg(unix)]
static MINT_CHILD: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
/// SIGTERM arrived before the child started; don't start it.
#[cfg(unix)]
static MINT_TERM: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Supervisors (`docker stop`, `kill <pid>`, some CI runners) may signal only
/// this process, so pass SIGTERM on to the child and stay alive to delete the
/// key. `kill` and atomics are async-signal-safe.
#[cfg(unix)]
extern "C" fn forward_term(sig: libc::c_int) {
    let pid = MINT_CHILD.load(std::sync::atomic::Ordering::SeqCst);
    if pid > 0 {
        unsafe {
            libc::kill(pid, sig);
        }
    } else {
        MINT_TERM.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

#[cfg(unix)]
impl MintSignals {
    fn ignore() -> Self {
        // SIGINT from a terminal already reaches the child through the process
        // group, so the parent only ignores it. SIGTERM is forwarded. The
        // child resets both to their defaults.
        MINT_CHILD.store(0, std::sync::atomic::Ordering::SeqCst);
        MINT_TERM.store(false, std::sync::atomic::Ordering::SeqCst);
        unsafe {
            Self(
                libc::signal(libc::SIGINT, libc::SIG_IGN),
                libc::signal(libc::SIGTERM, forward_term as *const () as libc::sighandler_t),
            )
        }
    }

    fn terminated_before_start() -> bool {
        MINT_TERM.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn track(child: &std::process::Child) {
        MINT_CHILD.store(child.id() as i32, std::sync::atomic::Ordering::SeqCst);
        // A SIGTERM between the check above and here would have been recorded
        // but not forwarded.
        if MINT_TERM.load(std::sync::atomic::Ordering::SeqCst) {
            unsafe {
                libc::kill(child.id() as libc::pid_t, libc::SIGTERM);
            }
        }
    }
}

#[cfg(unix)]
impl Drop for MintSignals {
    fn drop(&mut self) {
        unsafe {
            libc::signal(libc::SIGINT, self.0);
            libc::signal(libc::SIGTERM, self.1);
        }
        MINT_CHILD.store(0, std::sync::atomic::Ordering::SeqCst);
    }
}

#[cfg(windows)]
struct MintSignals;

#[cfg(windows)]
impl MintSignals {
    fn ignore() -> Self {
        // Console events reach the parent and child. Keep the parent alive to
        // delete the key; the child still receives the event normally.
        unsafe { set_console_ctrl_handler(Some(mint_console_event), 1) };
        Self
    }

    // Windows has no SIGTERM; console events reach the child directly.
    fn terminated_before_start() -> bool {
        false
    }

    fn track(_: &std::process::Child) {}
}

#[cfg(windows)]
impl Drop for MintSignals {
    fn drop(&mut self) {
        unsafe { set_console_ctrl_handler(Some(mint_console_event), 0) };
    }
}

#[cfg(windows)]
unsafe extern "system" fn mint_console_event(event: u32) -> i32 {
    if matches!(event, 0 | 1) { 1 } else { 0 } // CTRL_C_EVENT or CTRL_BREAK_EVENT
}

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    #[link_name = "SetConsoleCtrlHandler"]
    fn set_console_ctrl_handler(handler: Option<unsafe extern "system" fn(u32) -> i32>, add: i32) -> i32;
}

/// A client pointed at the Platform API, which takes the same bearer key as
/// the inference API.
pub(crate) fn platform_client(m: &ArgMatches, base: &str) -> anyhow::Result<Client> {
    let mut client = new_client(m)?;
    if client.config.auth_header.is_empty() {
        return Err(exit::err(
            Code::Auth,
            format!(
                "no API key configured\nhint: create a key with the api-read or api-admin permission at {}\n      then export MIXLAYER_API_KEY or run: pbpaste | mixlayer auth set-token",
                console_url("/app/api-keys", "keys")
            ),
        ));
    }
    client.config.base_url = base.to_owned();
    Ok(client)
}

/// `--org`, or the single organization the key can see.
pub(crate) fn org_id(m: &ArgMatches, g: &Globals, client: &mut Client) -> anyhow::Result<String> {
    if let Some(org) = arg(m, "org").filter(|s| !s.is_empty()) {
        return Ok(org.to_owned());
    }
    let orgs = client
        .get_no_cache("/v1/organizations")
        .map_err(|e| platform_error(e, g, "api-read"))?;
    let ids: Vec<String> = match data(orgs) {
        Value::Array(items) => items
            .iter()
            .filter_map(|o| o.get("id").and_then(Value::as_str).map(str::to_owned))
            .collect(),
        _ => Vec::new(),
    };
    match ids.as_slice() {
        [one] => Ok(one.clone()),
        [] => Err(exit::err(Code::NotFound, "this key cannot see any organization")),
        many => Err(exit::usage(format!(
            "this key can see {} organizations; pass --org with one of: {}",
            many.len(),
            many.join(", ")
        ))),
    }
}

pub(crate) fn permissions(m: &ArgMatches) -> anyhow::Result<Vec<String>> {
    let given: Vec<String> = m
        .get_many::<String>("permissions")
        .map(|v| v.map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()).collect())
        .unwrap_or_default();
    if given.is_empty() {
        return Ok(vec!["inference".into()]);
    }
    let mut out: Vec<String> = Vec::new();
    for p in given {
        if !PERMISSIONS.contains(&p.as_str()) {
            return Err(exit::usage(format!(
                "unknown permission {p:?} (choose from {})",
                PERMISSIONS.join(", ")
            )));
        }
        if !out.contains(&p) {
            out.push(p);
        }
    }
    Ok(out)
}

fn key_id(m: &ArgMatches, command: &str) -> anyhow::Result<String> {
    let args = words(m, command);
    match args.as_slice() {
        [id] if !id.trim().is_empty() => Ok(id.trim().to_owned()),
        [] => Err(exit::usage("key-id is required (see `mixlayer keys list`)")),
        _ => Err(exit::usage("expected exactly one key-id")),
    }
}

pub(crate) fn keys_path(org: &str) -> String {
    format!("/v1/organizations/{}/api_keys", encode(org))
}

pub(crate) fn key_path(org: &str, id: &str) -> String {
    format!("{}/{}", keys_path(org), encode(id))
}

pub(crate) fn encode(segment: &str) -> String {
    if segment == ORG_PLACEHOLDER {
        return segment.to_owned();
    }
    let mut out = String::new();
    for b in segment.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Platform responses wrap their payload as `{"data": ...}`.
pub(crate) fn data(v: Value) -> Value {
    match v {
        Value::Object(mut obj) if obj.contains_key("data") => obj.remove("data").unwrap_or(Value::Null),
        other => other,
    }
}

pub(crate) fn dry_run(method: &str, base: &str, path: &str, body: Option<&Value>) -> Value {
    let mut v = json!({"dry_run": true, "method": method, "url": format!("{base}{path}")});
    if let Some(body) = body {
        v["body"] = body.clone();
    }
    v
}

/// Ask before a destructive call. Only an explicit `--yes` skips the question:
/// the `--yes` that `--agent` mode implies does not, so an agent has to write
/// `--yes` into the command, where the user sees it. Without a terminal the
/// command refuses.
pub(crate) fn confirm(m: &ArgMatches, g: &Globals, question: &str) -> anyhow::Result<()> {
    let explicit_yes = m.value_source("yes") == Some(clap::parser::ValueSource::CommandLine)
        && m.try_get_one::<bool>("yes").ok().flatten() == Some(&true);
    if explicit_yes {
        return Ok(());
    }
    if g.no_input || !std::io::stdin().is_terminal() {
        return Err(exit::usage(format!("{question}\nPass --yes to confirm.")));
    }
    eprint!("{question} [y/N] ");
    std::io::stderr().flush()?;
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    if matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        Ok(())
    } else {
        Err(exit::err(Code::Other, "cancelled"))
    }
}

/// Show or save a one-time secret. The secret goes to stdout alone (or in the
/// JSON object), and the explanation to stderr, so `> file` captures only the key.
fn deliver_secret(m: &ArgMatches, g: &Globals, mut key: Value, verb: &str) -> anyhow::Result<()> {
    let secret = key
        .get("key")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| exit::err(Code::Api, "the API response did not include the new secret"))?;
    let id = key.get("id").and_then(Value::as_str).unwrap_or("").to_owned();
    let perms = key
        .get("permissions")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", "))
        .unwrap_or_default();
    if bool_arg(m, "save") {
        let cfg = config::load(arg(m, "config")).map_err(|e| exit::err(Code::Config, e.to_string()))?;
        cfg.save_token(&secret)
            .map_err(|e| exit::err(Code::Config, format!("saving token: {e}")))?;
        if let Some(obj) = key.as_object_mut() {
            obj.remove("key");
            obj.insert("saved_to".into(), cfg.credentials_path.display().to_string().into());
        }
        if g.json {
            return print(&key, g);
        }
        println!(
            "{verb} API key {id} ({perms}) and saved it to {}",
            cfg.credentials_path.display()
        );
        if std::env::var("MIXLAYER_API_KEY").is_ok_and(|v| !v.is_empty()) {
            println!("Note: MIXLAYER_API_KEY is set in this shell and takes priority over the saved key.");
        }
        return Ok(());
    }
    if g.json {
        return print(&key, g);
    }
    if !g.quiet {
        eprintln!("{verb} API key {id} ({perms}). This is the only time the secret is shown:");
    }
    crate::output::write_stdout(format!("{secret}\n").as_bytes())
}

/// Auth and not-found errors from the Platform API name the permission the
/// call needs instead of the inference hints.
pub(crate) fn platform_error(err: anyhow::Error, g: &Globals, needs: &str) -> anyhow::Error {
    let Some(api) = err.downcast_ref::<ApiError>() else {
        return classified(err, g, "keys");
    };
    let keys_page = console_url("/app/api-keys", "keys");
    match api.status {
        403 => exit::err(
            Code::Auth,
            format!(
                "{err}\nhint: this needs an API key with the {needs} permission.\n      Edit the key's permissions at {keys_page}"
            ),
        ),
        401 => exit::err(
            Code::Auth,
            format!("{err}\nhint: the key was rejected. Check MIXLAYER_API_KEY, or create a key at {keys_page}"),
        ),
        404 => exit::err(
            Code::NotFound,
            format!("{err}\nhint: check the ID with `mixlayer keys list` (and --org if you have several)"),
        ),
        _ => classified(err, g, "keys"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_unwraps_platform_envelope() {
        assert_eq!(data(json!({"data": [1, 2]})), json!([1, 2]));
        assert_eq!(data(json!([3])), json!([3]));
    }

    #[test]
    fn paths_encode_segments() {
        assert_eq!(keys_path("org_1"), "/v1/organizations/org_1/api_keys");
        assert_eq!(key_path("o/x", "k 1"), "/v1/organizations/o%2Fx/api_keys/k%201");
        assert_eq!(keys_path(ORG_PLACEHOLDER), "/v1/organizations/<org-id>/api_keys");
    }
}
