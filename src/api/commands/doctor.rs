use std::time::Duration;

use clap::ArgMatches;
use serde_json::{Value, json};

use super::{arg, bool_arg, print};
use crate::Globals;
use crate::api::{
    client::{ApiError, Client, parse_duration},
    config, console_url,
};
use crate::exit::{self, Code};
use crate::paths::{self, Kind};

fn paths_report() -> Value {
    let mut out = json!({"status": "ok"});
    for kind in [Kind::Config, Kind::Data, Kind::State, Kind::Cache] {
        match paths::resolve(kind) {
            Ok(r) => {
                out[kind.name()] = json!({"dir": r.dir, "rung": r.rung, "source": r.source});
            }
            Err(e) => {
                out["status"] = "error".into();
                out["detail"] = e.to_string().into();
            }
        }
    }
    out
}

fn interstitial(body: &str) -> Option<&'static str> {
    let lower = body.chars().take(8192).collect::<String>().to_ascii_lowercase();
    if !lower.contains("<title") {
        return None;
    }
    if lower.contains("<title>just a moment") || lower.contains("challenges.cloudflare.com") {
        return Some("Cloudflare");
    }
    if lower.contains("akamai") && (lower.contains("request unsuccessful") || lower.contains("access denied")) {
        return Some("Akamai");
    }
    if lower.contains("vercel") && lower.contains("challenge") {
        return Some("Vercel");
    }
    if lower.contains("request blocked") && lower.contains("aws waf") {
        return Some("AWS WAF");
    }
    if lower.contains("datadome") && lower.contains("captcha") {
        return Some("DataDome");
    }
    if lower.contains("perimeterx") || lower.contains("px-captcha") {
        return Some("PerimeterX");
    }
    None
}

pub fn run(m: &ArgMatches, g: &Globals) -> anyhow::Result<()> {
    let mut report =
        json!({"paths": paths_report(), "verify_mode": "normal operation", "version": crate::cli::VERSION});
    let cfg = config::load(arg(m, "config"));
    match cfg {
        Ok(cfg) => {
            report["config"] = "ok".into();
            report["config_path"] = cfg.path.display().to_string().into();
            report["base_url"] = cfg.base_url.clone().into();
            report["credentials_location"] = if cfg.credential_source.is_empty() {
                "none".into()
            } else {
                cfg.credential_source.clone().into()
            };
            let has_auth = !cfg.auth_header.is_empty();
            report["auth"] = if has_auth {
                "configured".into()
            } else {
                "not configured".into()
            };
            if has_auth {
                report["auth_source"] = cfg.auth_source.clone().into();
            } else {
                report["auth_hint"] = format!(
                    "Set it with: export MIXLAYER_API_KEY=\"your-key\" (create one at {})",
                    console_url("/sign-up", "doctor")
                )
                .into();
            }
            report["env_vars"] = if std::env::var("MIXLAYER_API_KEY").is_ok_and(|v| !v.is_empty()) {
                "OK 1/1 available".into()
            } else if has_auth {
                format!("OK credentials available from {}", cfg.auth_source).into()
            } else {
                "ERROR missing required: MIXLAYER_API_KEY".into()
            };
            let requested = parse_duration(arg(m, "timeout").unwrap_or("1m0s")).unwrap_or(Duration::from_secs(60));
            let timeout = if requested.is_zero() {
                Duration::from_secs(10)
            } else {
                requested.min(Duration::from_secs(10))
            };
            let rate = arg(m, "rate-limit").unwrap_or("0").parse().unwrap_or(0.0);
            match Client::new(cfg, timeout, rate, bool_arg(m, "no-cache")) {
                Ok(mut client) => {
                    let root = client.probe("/");
                    let reached = root.is_ok() || root.as_ref().is_err_and(|e| e.downcast_ref::<ApiError>().is_some());
                    report["api"] = if reached {
                        let body = match &root {
                            Ok(v) => v.to_string(),
                            Err(e) => e.to_string(),
                        };
                        match interstitial(&body) {
                            Some(vendor) => format!("blocked by {vendor} interstitial").into(),
                            None => "reachable".into(),
                        }
                    } else {
                        format!("unreachable: {}", root.unwrap_err()).into()
                    };
                    if has_auth {
                        if !reached {
                            report["credentials"] = "skipped (API unreachable)".into();
                        } else {
                            match client.probe("/models") {
                                Ok(_) => {
                                    report["auth"] = "valid".into();
                                    report["credentials"] = "verified via GET /models".into();
                                }
                                Err(e)
                                    if e.downcast_ref::<ApiError>()
                                        .is_some_and(|a| a.status == 401 || a.status == 403) =>
                                {
                                    let status = e.downcast_ref::<ApiError>().expect("checked").status;
                                    report["auth"] = "invalid".into();
                                    report["credentials"] = format!("invalid (HTTP {status} at /models)").into();
                                }
                                Err(e) => {
                                    report["credentials"] = format!("present, not verified: {e}").into();
                                }
                            }
                        }
                    }
                }
                Err(e) => report["api"] = format!("client init error: {e}").into(),
            }
        }
        Err(e) => report["config"] = format!("error: {e}").into(),
    }
    let db = paths::dir(Kind::Data)?.join("data.db");
    report["cache"] = if db.exists() {
        json!({"status": "unknown", "db_path": db, "db_bytes": std::fs::metadata(&db).ok().map(|m| m.len())})
    } else {
        json!({"status": "unknown", "db_path": db, "hint": "Local ledger not created yet; it is created on the first ask, ladder or shield run."})
    };
    let skills = crate::agent_setup::managed_skill_report();
    if !skills.is_empty() {
        if skills.iter().any(|s| s["status"] == "outdated") {
            report["agent_skills_hint"] =
                "An agent has an older Mixlayer skill than this CLI. Update it with: mixlayer agent setup --yes".into();
        }
        report["agent_skills"] = Value::Array(skills);
    }
    if g.json {
        print(&report, g)?;
    } else {
        for (key, label) in [
            ("config", "Config"),
            ("auth", "Auth"),
            ("env_vars", "Env Vars"),
            ("api", "API"),
            ("credentials", "Credentials"),
        ] {
            if let Some(value) = report[key].as_str() {
                println!("  {label}: {value}");
            }
        }
        for key in [
            "config_path",
            "base_url",
            "auth_source",
            "credentials_location",
            "version",
            "auth_hint",
        ] {
            if let Some(value) = report[key].as_str() {
                println!("  {key}: {value}");
            }
        }
        for kind in ["config", "data", "state", "cache"] {
            if let Some(value) = report["paths"][kind]["dir"].as_str() {
                println!("  {kind}: {value}");
            }
        }
        if let Some(skills) = report["agent_skills"].as_array() {
            for s in skills {
                println!(
                    "  skill ({}): {}",
                    s["agent"].as_str().unwrap_or(""),
                    s["status"].as_str().unwrap_or("")
                );
            }
        }
        if let Some(hint) = report["agent_skills_hint"].as_str() {
            println!("  {hint}");
        }
    }
    if let Some(gate) = arg(m, "fail-on").filter(|s| !s.is_empty()) {
        if !["error", "warn", "stale"].contains(&gate) {
            return Err(exit::err(
                Code::Other,
                format!("doctor: unknown --fail-on value {gate:?} (valid: stale, warn, error)"),
            ));
        }
        let serialized = report.to_string().to_ascii_lowercase();
        let error = ["error", "unreachable", "invalid", "missing"]
            .iter()
            .any(|s| serialized.contains(s));
        if error {
            return Err(exit::err(Code::Other, format!("doctor: --fail-on={gate} triggered")));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doctor_interstitial_detection_is_anchored() {
        assert_eq!(interstitial("<title>Just a Moment...</title>"), Some("Cloudflare"));
        assert_eq!(interstitial("Just a Moment of Pause Cookies"), None);
        assert_eq!(interstitial("<title>Recipe</title> Just a Moment"), None);
    }
}
