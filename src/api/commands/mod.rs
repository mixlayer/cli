mod account;
mod auth;
mod chat;
mod doctor;
pub mod keys;
pub(crate) mod models;

use clap::ArgMatches;
use serde_json::Value;

use crate::Globals;
use crate::api::client::{self, ApiError, Client};
use crate::api::config;
use crate::exit::{self, Code};

pub fn dispatch(path: &[&str], m: &ArgMatches, g: &Globals) -> Option<anyhow::Result<()>> {
    Some(match path {
        ["auth", "setup"] => auth::setup(m),
        ["auth", "set-token"] => auth::set_token(m, g),
        ["auth", "status"] => auth::status(m, g),
        ["auth", "logout"] => auth::logout(m, g),
        ["doctor"] => doctor::run(m, g),
        ["keys", "list"] => keys::list(m, g),
        ["keys", "create"] => keys::create(m, g),
        ["keys", "rotate"] => keys::rotate(m, g),
        ["keys", "delete"] => keys::delete(m, g),
        ["keys", "mint"] => keys::mint(m, g),
        ["whoami"] => account::whoami(m, g),
        ["spend"] => account::spend(m, g),
        ["budget"] | ["budget", "show"] => account::budget_show(m, g),
        ["budget", "set"] => account::budget_set(m, g),
        ["audit"] => account::audit(m, g),
        ["models", "list"] => models::list(m, g),
        ["models", "get"] => models::get(m, g),
        ["models", "query"] => models::query(m, g),
        ["chat"] => chat::chat(m, g),
        ["ask"] => chat::ask(m, g),
        ["shield", "ask"] if g.dry_run => print(
            &serde_json::json!({"dry_run": true, "endpoint": "/chat/completions"}),
            g,
        ),
        _ => return None,
    })
}

pub(super) fn arg<'a>(m: &'a ArgMatches, name: &str) -> Option<&'a str> {
    m.get_one::<String>(name).map(String::as_str)
}

pub(super) fn bool_arg(m: &ArgMatches, name: &str) -> bool {
    m.try_get_one::<bool>(name).ok().flatten().copied().unwrap_or(false)
}

pub(super) fn words(m: &ArgMatches, command: &str) -> Vec<String> {
    crate::cli::positional_words(crate::spec_types::find(command).expect("command in spec"), m)
}

pub(crate) fn new_client(m: &ArgMatches) -> anyhow::Result<Client> {
    let timeout = client::parse_duration(arg(m, "timeout").unwrap_or("1m0s"))
        .map_err(|e| exit::usage(format!("invalid --timeout: {e}")))?;
    new_client_with_timeout(m, timeout)
}

/// Like `new_client` with an explicit per-request timeout (shield detectors
/// use `--detector-timeout`).
pub(crate) fn new_client_with_timeout(m: &ArgMatches, timeout: std::time::Duration) -> anyhow::Result<Client> {
    let cfg = config::load(arg(m, "config")).map_err(|e| exit::err(Code::Config, e.to_string()))?;
    let rate = arg(m, "rate-limit")
        .unwrap_or("0")
        .parse::<f64>()
        .map_err(|e| exit::usage(format!("invalid --rate-limit: {e}")))?;
    if !rate.is_finite() || rate < 0.0 || (rate > 0.0 && std::time::Duration::try_from_secs_f64(1.0 / rate).is_err()) {
        return Err(exit::usage(format!(
            "invalid --rate-limit {rate}: use a number of requests per second, or 0 to disable"
        )));
    }
    Client::new(cfg, timeout, rate, bool_arg(m, "no-cache"))
}

pub(crate) fn classified(err: anyhow::Error, g: &Globals, campaign: &str) -> anyhow::Error {
    let api = err.downcast_ref::<ApiError>();
    let status = api.map(|e| e.status);
    let raw = err.to_string();
    let lower = raw.to_ascii_lowercase();
    let code = match status {
        Some(401 | 403) => Code::Auth,
        Some(402) => Code::Billing,
        Some(404) => Code::NotFound,
        Some(429) => Code::RateLimited,
        Some(400) if lower.contains("model not found") || lower.contains("model_not_found") => Code::NotFound,
        Some(400) if lower.contains("invalid_api_key") || lower.contains("unauthorized") => Code::Auth,
        _ => Code::Api,
    };
    let setup = crate::api::console_url("/sign-up", campaign);
    let billing = crate::api::console_url("/app/billing", campaign);
    let hint = match (code, status) {
        (Code::Auth, Some(403)) => format!(
            "\nhint: permission denied. Your credentials are valid but lack access to this resource.\n      Check that your credentials have the required permissions and match the API's expected auth scheme.\n      Set it with: export MIXLAYER_API_KEY=\"your-key\" (create one at {setup})\n      Run 'mixlayer doctor' to check auth status"
        ),
        (Code::Auth, Some(400)) => format!(
            "\nhint: the API rejected the request — this usually means auth is missing or invalid.\n      Set it with: export MIXLAYER_API_KEY=\"your-key\" (create one at {setup})\n      Run 'mixlayer doctor' to check auth status.\n      Response: {raw}"
        ),
        (Code::Auth, _) => format!(
            "\nhint: check your token.\n      Set it with: export MIXLAYER_API_KEY=\"your-key\" (create one at {setup})\n      Run 'mixlayer doctor' to check auth status"
        ),
        (Code::Billing, _) => format!("\nhint: add prepaid credits at {billing}; do not retry"),
        (Code::NotFound, _) => "\nhint: run `mixlayer models list` to see models available to your key".into(),
        (Code::RateLimited, _) => match api.map(|e| e.retry_after.as_str()).filter(|s| !s.is_empty()) {
            Some(after) => format!("\nhint: rate limited; Retry-After: {after}"),
            None => "\nhint: rate limited; honor Retry-After before retrying".into(),
        },
        _ => String::new(),
    };
    let message = format!("{raw}{hint}");
    if g.json && status == Some(409) {
        let _ = crate::output::print_json(
            &serde_json::json!({"error": message, "code": code as u8}),
            &Globals::default(),
        );
    }
    exit::err(code, message)
}

pub(super) fn print(v: &Value, g: &Globals) -> anyhow::Result<()> {
    crate::output::print_json(v, g)
}

pub(super) fn as_results(mut data: Value, g: &Globals) -> Value {
    if let Value::Object(obj) = &data
        && obj.len() == 1
        && let Some(array) = ["results", "data", "items", "nodes", "entries", "records"]
            .iter()
            .find_map(|k| obj.get(*k))
        && array.is_array()
    {
        data = array.clone();
    }
    let filtered = crate::output::filtered(data, g);
    serde_json::json!({"results": filtered, "meta": {"source": "live"}})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_statuses_keep_go_exit_codes_and_attributed_hints() {
        for (status, code) in [
            (401, Code::Auth),
            (403, Code::Auth),
            (402, Code::Billing),
            (404, Code::NotFound),
            (429, Code::RateLimited),
            (500, Code::Api),
        ] {
            let api = ApiError {
                method: "POST".into(),
                path: "/chat/completions".into(),
                status,
                body: "fake".into(),
                retry_after: "2".into(),
            };
            let err = classified(api.into(), &Globals::default(), "ask");
            assert_eq!(exit::code_of(&err), code);
            if status == 401 {
                assert!(err.to_string().contains("utm_campaign=ask"));
            }
            if status == 429 {
                assert!(err.to_string().contains("Retry-After: 2"));
            }
        }
    }
}
