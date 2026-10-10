// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

//! Read and manage the account behind the configured Platform API key.

use std::time::Duration;

use clap::ArgMatches;
use serde_json::{Value, json};

use super::{arg, print};
use crate::Globals;
use crate::api::client::{self, ApiError, Client};
use crate::api::commands::keys::{confirm, data, dry_run, encode, keys_path, org_id, platform_client, platform_error};
use crate::api::platform_base;
use crate::exit::{self, Code};

fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

fn number(v: &Value, key: &str) -> f64 {
    v.get(key).and_then(Value::as_f64).unwrap_or(0.0)
}

fn usd(cents: f64) -> f64 {
    cents / 100.0
}

fn money(value: f64) -> String {
    let sign = if value < 0.0 { "-" } else { "" };
    if value != 0.0 && value.abs() < 0.01 {
        format!("{sign}<$0.01")
    } else {
        format!("{sign}${:.2}", value.abs())
    }
}

fn billing(m: &ArgMatches, g: &Globals, client: &mut Client) -> anyhow::Result<String> {
    let id = org_id(m, g, client)?;
    let organizations = data(
        client
            .get_no_cache("/v1/organizations")
            .map_err(|e| platform_error(e, g, "api-read"))?,
    );
    organizations
        .as_array()
        .and_then(|a| a.iter().find(|o| text(o, "id") == id))
        .and_then(|o| o.get("billing_account_id"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| exit::err(Code::NotFound, format!("organization {id} has no billing account")))
}

pub fn whoami(m: &ArgMatches, g: &Globals) -> anyhow::Result<()> {
    let base = platform_base()?;
    let mut client = platform_client(m, &base)?;
    let source = if client.config.credential_source.is_empty() {
        &client.config.auth_source
    } else {
        &client.config.credential_source
    }
    .clone();
    let key_id = client
        .config
        .auth_header
        .strip_prefix("Bearer ")
        .and_then(|s| s.strip_prefix("sk_mxl_"))
        .and_then(|s| s.split_once('_'))
        .map(|(id, _)| id.to_owned())
        .filter(|s| !s.is_empty());
    let orgs = match client.get_no_cache("/v1/organizations") {
        Ok(v) => data(v),
        Err(e) if e.downcast_ref::<ApiError>().is_some_and(|a| a.status == 403) => {
            let report = json!({"key_id": key_id, "key_source": source, "platform_base_url": base,
                "platform_access": "none", "can": {"inference": null, "read": false, "write": false, "admin": false},
                "note": "this key cannot read the Platform API (needs api-read); it may still run models"});
            return show_identity(&report, g);
        }
        Err(e) => return Err(platform_error(e, g, "api-read")),
    };
    let visible: Vec<&Value> = orgs
        .as_array()
        .into_iter()
        .flatten()
        .filter(|o| arg(m, "org").is_none_or(|id| text(o, "id") == id))
        .collect();
    if visible.is_empty() && arg(m, "org").is_some() {
        return Err(exit::err(Code::NotFound, "organization not visible to this key"));
    }
    let mut found = None;
    if let Some(id) = key_id.as_deref() {
        for org in &visible {
            let keys = data(
                client
                    .get_no_cache(&keys_path(text(org, "id")))
                    .map_err(|e| platform_error(e, g, "api-read"))?,
            );
            if let Some(key) = keys.as_array().and_then(|a| a.iter().find(|k| text(k, "id") == id)) {
                found = Some(((*org).clone(), key.clone()));
                break;
            }
        }
    }
    let mut report = json!({"key_id": key_id, "key_source": source, "platform_base_url": base,
        "platform_access": "read", "can": {"inference": null, "read": true, "write": false, "admin": false}});
    if let Some(org) = visible.first() {
        report["organization"] = json!({"id": org["id"], "name": org["name"], "role": org["role"],
            "is_personal": org["is_personal"], "billing_account_id": org["billing_account_id"]});
    }
    if let Some((org, key)) = found {
        let permissions: Vec<&str> = key["permissions"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        let has = |scope| permissions.contains(&scope);
        let admin = has("api-admin");
        let write = admin || has("api-write");
        report["can"] = json!({"inference": has("inference"), "read": true, "write": write, "admin": admin});
        report["platform_access"] = (if admin {
            "admin"
        } else if write {
            "write"
        } else {
            "read"
        })
        .into();
        for field in ["name", "permissions", "expires_at", "active"] {
            report[field] = key[field].clone();
        }
        report["organization"] = json!({"id": org["id"], "name": org["name"], "role": org["role"],
            "is_personal": org["is_personal"], "billing_account_id": org["billing_account_id"]});
    } else {
        // Reading worked, but without the key's own record its write and
        // admin rights are unknown.
        report["can"]["write"] = Value::Null;
        report["can"]["admin"] = Value::Null;
        report["note"] = if key_id.is_none() {
            "key ID unknown (legacy key); cannot match an API key record"
        } else {
            "key ID was not found in visible organizations"
        }
        .into();
    }
    show_identity(&report, g)
}

fn show_identity(v: &Value, g: &Globals) -> anyhow::Result<()> {
    if g.json {
        return print(v, g);
    }
    let field = |key| v.get(key).and_then(Value::as_str).unwrap_or("unknown");
    println!("{:<14} {}", "Key", field("key_id"));
    println!("{:<14} {}", "Name", field("name"));
    println!(
        "{:<14} {}",
        "Permissions",
        v["permissions"]
            .as_array()
            .map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", "))
            .unwrap_or_else(|| "unknown".into())
    );
    println!(
        "{:<14} {}",
        "Expires",
        if v.get("name").is_some() && v["expires_at"].is_null() {
            "never"
        } else {
            field("expires_at")
        }
    );
    println!("{:<14} {}", "Organization", text(&v["organization"], "name"));
    println!("{:<14} {}", "Role", text(&v["organization"], "role"));
    println!("{:<14} {}", "Source", field("key_source"));
    if let Some(note) = v.get("note").and_then(Value::as_str) {
        println!("Note: {note}");
    }
    Ok(())
}

/// Usage per model, largest first. Invoice credits arrive as a group with no
/// model and a negative total; they are reported as `credits_usd`, not here.
fn by_model(groups: &[Value]) -> Value {
    let mut rows: Vec<Value> = groups
        .iter()
        .filter(|g| g.get("model").and_then(Value::as_str).is_some() || number(g, "total_cents") > 0.0)
        .map(|g| {
            json!({"model": g.get("model").and_then(Value::as_str).unwrap_or("other"),
        "usd": usd(number(g, "total_cents"))})
        })
        .collect();
    rows.sort_by(|a, b| number(b, "usd").total_cmp(&number(a, "usd")));
    Value::Array(rows)
}

pub fn spend(m: &ArgMatches, g: &Globals) -> anyhow::Result<()> {
    let range = arg(m, "range").unwrap_or("current");
    if !["current", "30d", "90d", "ytd"].contains(&range) {
        return Err(exit::usage("--range must be current, 30d, 90d or ytd"));
    }
    let threshold = arg(m, "fail-over")
        .map(|s| s.parse::<f64>().map_err(|_| exit::usage("invalid --fail-over")))
        .transpose()?;
    if threshold.is_some_and(|v| !v.is_finite() || v < 0.0) {
        return Err(exit::usage("--fail-over must be a nonnegative dollar amount"));
    }
    let base = platform_base()?;
    let mut client = platform_client(m, &base)?;
    let ba = billing(m, g, &mut client)?;
    let prefix = format!("/v1/billing_accounts/{}", encode(&ba));
    let report = if range == "current" {
        let invoices = data(
            client
                .get_no_cache(&format!("{prefix}/usage/current"))
                .map_err(|e| platform_error(e, g, "api-read"))?,
        );
        let invoice = invoices
            .as_array()
            .and_then(|a| a.first())
            .cloned()
            .unwrap_or(Value::Null);
        let controls = data(
            client
                .get_no_cache(&format!("{prefix}/spend_controls"))
                .map_err(|e| platform_error(e, g, "api-read"))?,
        );
        let usage = usd(number(&invoice, "subtotal_cents"));
        let limit = controls["usage_spend_cutoff"]["threshold_cents"].as_f64().map(usd);
        let warning = controls["usage_spend_warning"]["threshold_cents"].as_f64().map(usd);
        json!({"range": "current", "period_start": invoice["start_date"], "period_end": invoice["end_date"],
            "usage_usd": usage, "credits_usd": usd(number(&invoice, "total_applied_credits_cents")),
            "net_usd": usd(number(&invoice, "total_amount_cents")), "by_model": by_model(invoice["line_item_groups"].as_array().map(Vec::as_slice).unwrap_or(&[])),
            "limit_usd": limit, "warning_usd": warning, "state": controls["usage_spend_cutoff"]["local_state"],
            "limit_remaining_usd": limit.map(|n| n - usage)})
    } else {
        let raw = data(
            client
                .get_no_cache(&format!("{prefix}/usage_spend?range={range}"))
                .map_err(|e| platform_error(e, g, "api-read"))?,
        );
        let models = raw["models"].as_array().map(Vec::as_slice).unwrap_or(&[]);
        let daily: Vec<Value> = raw["buckets"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|b| json!({"date": b["date"], "usd": usd(number(b, "total_cents"))}))
            .collect();
        json!({"range": range, "range_start": raw["range_start"], "range_end": raw["range_end"],
            "usage_usd": usd(models.iter().map(|v| number(v, "total_cents")).sum()), "by_model": by_model(models), "daily": daily})
    };
    if g.json {
        print(&report, g)?;
    } else {
        if range == "current" {
            println!(
                "Period: {} to {}",
                text(&report, "period_start"),
                text(&report, "period_end")
            );
        } else {
            println!(
                "Range: {} to {}",
                text(&report, "range_start"),
                text(&report, "range_end")
            );
        }
        for row in report["by_model"].as_array().into_iter().flatten() {
            println!("{:<36} {}", text(row, "model"), money(number(row, "usd")));
        }
        println!("Usage: {}", money(number(&report, "usage_usd")));
        if range == "current" {
            println!(
                "Credits: {}  Net: {}",
                money(number(&report, "credits_usd")),
                money(number(&report, "net_usd"))
            );
            if let Some(limit) = report["limit_usd"].as_f64() {
                println!(
                    "Monthly limit: {}  Remaining: {}  State: {}",
                    money(limit),
                    money(number(&report, "limit_remaining_usd")),
                    text(&report, "state")
                );
            }
            if let Some(warning) = report["warning_usd"].as_f64() {
                println!("Warning: {}", money(warning));
            }
        }
    }
    if let Some(over) = threshold
        && number(&report, "usage_usd") > over
    {
        return Err(exit::err(
            Code::GateMatched,
            format!(
                "spend {} is over --fail-over {}",
                money(number(&report, "usage_usd")),
                money(over)
            ),
        ));
    }
    Ok(())
}

fn cents(raw: &str, flag: &str) -> anyhow::Result<Option<i64>> {
    if matches!(raw, "off" | "none") {
        return Ok(None);
    }
    let (whole, fraction) = raw.split_once('.').unwrap_or((raw, ""));
    if whole.is_empty()
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || fraction.len() > 2
        || !fraction.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(exit::usage(format!(
            "{flag} must be positive dollars with at most two decimals, or off"
        )));
    }
    let dollars: i64 = whole.parse().map_err(|_| exit::usage(format!("{flag} is too large")))?;
    let fraction: i64 = if fraction.is_empty() {
        0
    } else {
        format!("{fraction:0<2}")
            .parse()
            .map_err(|_| exit::usage(format!("invalid {flag}")))?
    };
    let value = dollars
        .checked_mul(100)
        .and_then(|v| v.checked_add(fraction))
        .ok_or_else(|| exit::usage(format!("{flag} is too large")))?;
    if value == 0 {
        return Err(exit::usage(format!("{flag} must be greater than zero")));
    }
    Ok(Some(value))
}

fn current_value(v: &Value, field: &str) -> Value {
    v[field]["threshold_cents"].clone()
}

pub fn budget_show(m: &ArgMatches, g: &Globals) -> anyhow::Result<()> {
    let base = platform_base()?;
    let mut client = platform_client(m, &base)?;
    let ba = billing(m, g, &mut client)?;
    let prefix = format!("/v1/billing_accounts/{}", encode(&ba));
    let controls = data(
        client
            .get_no_cache(&format!("{prefix}/spend_controls"))
            .map_err(|e| platform_error(e, g, "api-read"))?,
    );
    let account = data(
        client
            .get_no_cache(&format!("{prefix}?include=all"))
            .map_err(|e| platform_error(e, g, "api-read"))?,
    );
    let report = json!({"billing_account_id": ba, "billing_mode": account["billing_mode"],
        "monthly_limit_usd": current_value(&controls, "usage_spend_cutoff").as_f64().map(usd),
        "warning_usd": current_value(&controls, "usage_spend_warning").as_f64().map(usd),
        "low_balance_warning_usd": current_value(&controls, "prepaid_low_balance_warning").as_f64().map(usd),
        "state": controls["usage_spend_cutoff"]["local_state"],
        "prepaid_balance_usd": if text(&account, "billing_mode") == "prepaid" { account["credit_balance_cents"].as_f64().map(usd) } else { None }});
    if g.json {
        return print(&report, g);
    }
    for (label, field) in [
        ("Monthly limit", "monthly_limit_usd"),
        ("Warning", "warning_usd"),
        ("Low balance warning", "low_balance_warning_usd"),
    ] {
        println!(
            "{label:<22} {}",
            report[field].as_f64().map(money).unwrap_or_else(|| "not set".into())
        );
    }
    println!(
        "{:<22} {}",
        "Cutoff state",
        report["state"].as_str().unwrap_or("not set")
    );
    if let Some(v) = report["prepaid_balance_usd"].as_f64() {
        println!("{:<22} {}", "Prepaid balance", money(v));
    }
    Ok(())
}

pub fn budget_set(m: &ArgMatches, g: &Globals) -> anyhow::Result<()> {
    let flags = [
        ("monthly-limit", "usage_spend_cutoff_threshold_cents"),
        ("warn-at", "usage_spend_warning_threshold_cents"),
        ("low-balance-warning", "prepaid_low_balance_warning_threshold_cents"),
    ];
    if flags.iter().all(|(f, _)| arg(m, f).is_none()) {
        return Err(exit::usage("budget set needs at least one limit flag"));
    }
    let changed: Vec<Option<Option<i64>>> = flags
        .iter()
        .map(|(f, _)| arg(m, f).map(|v| cents(v, &format!("--{f}"))).transpose())
        .collect::<anyhow::Result<Vec<_>>>()?;
    let base = platform_base()?;
    if !g.dry_run {
        confirm(
            m,
            g,
            "Change billing spend controls? The monthly limit pauses paid model traffic when reached.",
        )?;
    }
    let mut maybe_client = match platform_client(m, &base) {
        Ok(client) => Some(client),
        Err(e)
            if g.dry_run && exit::code_of(&e) == Code::Auth && e.to_string().starts_with("no API key configured") =>
        {
            None
        }
        Err(e) => return Err(e),
    };
    let (ba, before) = if let Some(ref mut client) = maybe_client {
        let ba = billing(m, g, client)?;
        let path = format!("/v1/billing_accounts/{}/spend_controls", encode(&ba));
        let before = data(
            client
                .get_no_cache(&path)
                .map_err(|e| platform_error(e, g, "api-read"))?,
        );
        (ba, before)
    } else {
        ("<billing-account-id>".into(), Value::Null)
    };
    let old = [
        current_value(&before, "usage_spend_cutoff"),
        current_value(&before, "usage_spend_warning"),
        current_value(&before, "prepaid_low_balance_warning"),
    ];
    let mut body = serde_json::Map::new();
    for (i, (_, key)) in flags.iter().enumerate() {
        body.insert(
            (*key).into(),
            match changed[i] {
                Some(Some(n)) => json!(n),
                Some(None) => Value::Null,
                None if before.is_null() => Value::String("<current>".into()),
                None => old[i].clone(),
            },
        );
    }
    let body = Value::Object(body);
    if let (Some(warn), Some(limit)) = (
        body["usage_spend_warning_threshold_cents"].as_i64(),
        body["usage_spend_cutoff_threshold_cents"].as_i64(),
    ) && warn >= limit
    {
        return Err(exit::usage("--warn-at must be lower than --monthly-limit"));
    }
    let path = format!("/v1/billing_accounts/{}/spend_controls", encode(&ba));
    if g.dry_run {
        return print(
            &json!({"before": before, "after": body, "request": dry_run("PUT", &base, &path, Some(&body))}),
            g,
        );
    }
    let client = maybe_client.as_mut().expect("checked above");
    let after = data(
        client
            .put(&path, &body)
            .map_err(|e| platform_error(e, g, "api-admin"))?,
    );
    if g.json {
        print(&json!({"before": before, "after": after}), g)
    } else {
        for (label, field) in [
            ("Monthly limit", "usage_spend_cutoff"),
            ("Warning", "usage_spend_warning"),
            ("Low balance warning", "prepaid_low_balance_warning"),
        ] {
            let shown = |v: &Value| {
                current_value(v, field)
                    .as_f64()
                    .map(|n| money(usd(n)))
                    .unwrap_or_else(|| "not set".into())
            };
            println!("{label}: {} -> {}", shown(&before), shown(&after));
        }
        Ok(())
    }
}

fn audit_path(org: &str, m: &ArgMatches) -> anyhow::Result<String> {
    let limit: usize = arg(m, "limit")
        .unwrap_or("50")
        .parse()
        .map_err(|_| exit::usage("--limit must be 1–200"))?;
    if !(1..=200).contains(&limit) {
        return Err(exit::usage("--limit must be 1–200"));
    }
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    query.append_pair("limit", &limit.to_string());
    if let Some(since) = arg(m, "since") {
        if since.starts_with('-') {
            return Err(exit::usage("--since must be a positive duration or RFC3339 timestamp"));
        }
        let from = if let Ok(duration) = client::parse_duration(since) {
            (chrono::Utc::now() - chrono::Duration::from_std(duration).map_err(|_| exit::usage("invalid --since"))?)
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
        } else {
            chrono::DateTime::parse_from_rfc3339(since)
                .map_err(|_| exit::usage("--since must be a duration or RFC3339 timestamp"))?;
            since.to_owned()
        };
        query.append_pair("from", &from);
    }
    for (flag, key) in [
        ("action", "action"),
        ("actor", "actor"),
        ("resource-type", "resource_type"),
    ] {
        if let Some(v) = arg(m, flag) {
            query.append_pair(key, v);
        }
    }
    Ok(format!(
        "/v1/organizations/{}/audit_log?{}",
        encode(org),
        query.finish()
    ))
}

fn audit_line(entry: &Value) -> String {
    let meta = &entry["metadata"];
    let summary = if text(entry, "action").starts_with("api_key.") {
        format!(
            "{} {}",
            text(meta, "name"),
            meta["permissions"]
                .as_array()
                .map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(","))
                .unwrap_or_default()
        )
    } else {
        meta.to_string()
    };
    let summary: String = summary.chars().take(80).collect();
    format!(
        "{}  {:<24} {:<28} {:<18} {}",
        text(entry, "created_at"),
        text(entry, "action"),
        format!("{}/{}", text(entry, "resource_type"), text(entry, "resource_id")),
        text(entry, "actor"),
        summary
    )
}

pub fn audit(m: &ArgMatches, g: &Globals) -> anyhow::Result<()> {
    let interval = client::parse_duration(arg(m, "interval").unwrap_or("5s"))
        .map_err(|e| exit::usage(format!("invalid --interval: {e}")))?;
    if interval < Duration::from_secs(1) {
        return Err(exit::usage("--interval must be at least 1s"));
    }
    let base = platform_base()?;
    let mut client = platform_client(m, &base)?;
    let org = org_id(m, g, &mut client)?;
    let path = audit_path(&org, m)?;
    let follow = super::bool_arg(m, "follow");
    let first = client
        .get_no_cache(&path)
        .map_err(|e| platform_error(e, g, "api-admin"))?;
    if !follow {
        if g.json {
            return print(&first, g);
        }
        for entry in first["data"].as_array().into_iter().flatten() {
            println!("{}", audit_line(entry));
        }
        return Ok(());
    }
    let _signal = FollowSignal::install();
    let emit = |page: &Value| -> anyhow::Result<()> {
        for entry in page["data"].as_array().into_iter().flatten().rev() {
            if g.json {
                println!("{}", serde_json::to_string(entry)?);
            } else {
                println!("{}", audit_line(entry));
            }
        }
        use std::io::Write;
        std::io::stdout().flush()?;
        Ok(())
    };
    emit(&first)?;
    let mut cursor = first["start_cursor"].as_str().map(str::to_owned);
    while !FOLLOW_STOP.load(std::sync::atomic::Ordering::Relaxed) {
        std::thread::sleep(interval);
        if FOLLOW_STOP.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }
        let poll_path = match cursor.as_ref() {
            Some(current) => format!(
                "{path}&before={}",
                url::form_urlencoded::byte_serialize(current.as_bytes()).collect::<String>()
            ),
            // Nothing matched yet, so there is no cursor: anything the plain
            // query returns now is new.
            None => path.clone(),
        };
        match client.get_no_cache(&poll_path) {
            Ok(page) => {
                if page["data"].as_array().is_some_and(|a| !a.is_empty()) {
                    emit(&page)?;
                    if let Some(next) = page["start_cursor"].as_str() {
                        cursor = Some(next.to_owned());
                    }
                }
            }
            Err(e)
                if e.downcast_ref::<ApiError>()
                    .is_some_and(|a| a.status == 401 || a.status == 403) =>
            {
                return Err(platform_error(e, g, "api-admin"));
            }
            // Server errors, rate limits and network failures pass; keep polling.
            Err(e)
                if e.downcast_ref::<ApiError>()
                    .is_none_or(|a| a.status >= 500 || a.status == 429) =>
            {
                eprintln!("audit follow: {e}")
            }
            Err(e) => return Err(platform_error(e, g, "api-admin")),
        }
    }
    Ok(())
}

static FOLLOW_STOP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(unix)]
extern "C" fn stop_follow(_: libc::c_int) {
    FOLLOW_STOP.store(true, std::sync::atomic::Ordering::Relaxed);
}

#[cfg(unix)]
struct FollowSignal(libc::sighandler_t);

#[cfg(unix)]
impl FollowSignal {
    fn install() -> Self {
        FOLLOW_STOP.store(false, std::sync::atomic::Ordering::Relaxed);
        unsafe {
            Self(libc::signal(
                libc::SIGINT,
                stop_follow as *const () as libc::sighandler_t,
            ))
        }
    }
}

#[cfg(unix)]
impl Drop for FollowSignal {
    fn drop(&mut self) {
        unsafe {
            libc::signal(libc::SIGINT, self.0);
        }
    }
}

#[cfg(windows)]
struct FollowSignal;

#[cfg(windows)]
impl FollowSignal {
    fn install() -> Self {
        FOLLOW_STOP.store(false, std::sync::atomic::Ordering::Relaxed);
        unsafe { set_console_ctrl_handler(Some(stop_follow), 1) };
        Self
    }
}

#[cfg(windows)]
impl Drop for FollowSignal {
    fn drop(&mut self) {
        unsafe { set_console_ctrl_handler(Some(stop_follow), 0) };
    }
}

#[cfg(windows)]
unsafe extern "system" fn stop_follow(event: u32) -> i32 {
    if matches!(event, 0 | 1) {
        // CTRL_C_EVENT or CTRL_BREAK_EVENT
        FOLLOW_STOP.store(true, std::sync::atomic::Ordering::Relaxed);
        1
    } else {
        0
    }
}

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    #[link_name = "SetConsoleCtrlHandler"]
    fn set_console_ctrl_handler(handler: Option<unsafe extern "system" fn(u32) -> i32>, add: i32) -> i32;
}
