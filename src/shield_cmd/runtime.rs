use std::cell::{Cell, RefCell};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use clap::ArgMatches;
use reqwest::blocking::Client as HttpClient;
use serde_json::{Value, json};
use url::Url;

use crate::Globals;
use crate::api::client::{ApiError, parse_duration};
use crate::api::commands;
use crate::exit::{self, Code};
use crate::shield::{self, DictionaryTerm, Entity, RedactionResult};
use crate::store::Store;

use super::arg;

pub const PLACEHOLDER_INSTRUCTION: &str = "Placeholders matching <KIND>_<hex> are stand-ins for private values. Repeat each placeholder exactly. Never alter or expand it, and never guess the real value.";
const MIXLAYER_NOTICE: &str =
    "raw text is sent to Mixlayer for detection; the frontier upstream receives masked text only";

#[derive(Clone, Debug)]
pub struct Upstream {
    pub name: String,
    pub base_url: String,
    pub key_env: String,
    pub host: String,
}

fn valid_env(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

pub fn key_from_env(name: &str, optional: bool) -> Result<String> {
    if name.is_empty() {
        return Ok(String::new());
    }
    if !valid_env(name) {
        return Err(exit::usage(format!("invalid key environment variable name {name:?}")));
    }
    let key = std::env::var(name).unwrap_or_default();
    if key.is_empty() && !optional {
        return Err(exit::err(
            Code::Auth,
            format!("missing key in environment variable {name}"),
        ));
    }
    Ok(key)
}

pub fn resolve_upstream(m: &ArgMatches) -> Result<Upstream> {
    let name = arg(m, "upstream").unwrap_or("mixlayer");
    let base_arg = arg(m, "upstream-base-url").unwrap_or("");
    let key_arg = arg(m, "upstream-key-env").unwrap_or("");
    let (mut base, mut key) = match name {
        "mixlayer" => ("https://mixlayer.ai/v1", "MIXLAYER_API_KEY"),
        "openai" => ("https://api.openai.com/v1", "OPENAI_API_KEY"),
        "anthropic" => ("https://api.anthropic.com/v1", "ANTHROPIC_API_KEY"),
        "openrouter" => ("https://openrouter.ai/api/v1", "OPENROUTER_API_KEY"),
        "custom" if !base_arg.is_empty() && !key_arg.is_empty() => (base_arg, key_arg),
        "custom" => {
            return Err(exit::usage(
                "custom upstream requires --upstream-base-url and --upstream-key-env",
            ));
        }
        _ => return Err(exit::usage(format!("unknown upstream {name:?}"))),
    };
    if !base_arg.is_empty() {
        if name != "custom" {
            return Err(exit::usage("--upstream-base-url requires --upstream custom"));
        }
        base = base_arg;
    }
    if !key_arg.is_empty() {
        key = key_arg;
    }
    if !valid_env(key) {
        return Err(exit::usage(format!("invalid key environment variable name {key:?}")));
    }
    let url = Url::parse(base).map_err(|_| exit::usage("invalid upstream base URL"))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(exit::usage("invalid upstream base URL"));
    }
    Ok(Upstream {
        name: name.into(),
        base_url: base.trim_end_matches('/').into(),
        key_env: key.into(),
        host: url[url::Position::BeforeHost..url::Position::AfterPort].to_string(),
    })
}

#[derive(Default, Clone, Debug)]
pub struct Usage {
    pub prompt: i64,
    pub completion: i64,
    pub cache: i64,
    pub unknown: bool,
}

#[derive(Clone, Debug)]
pub struct ChatResult {
    pub answer: String,
    pub reasoning: String,
    pub raw: Value,
    pub usage: Value,
    pub latency_ms: i64,
}

fn response_chat(raw: Value, latency_ms: i64) -> Result<ChatResult> {
    let answer = raw["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("")
        .to_string();
    if answer.is_empty() {
        bail!("upstream chat response has no answer");
    }
    let reasoning = raw["choices"][0]["message"]["reasoning_content"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let usage = raw["usage"].clone();
    Ok(ChatResult {
        answer,
        reasoning,
        raw,
        usage,
        latency_ms,
    })
}

pub fn external_chat(base: &str, key: &str, body: &Value, timeout: Duration) -> Result<ChatResult> {
    let http = HttpClient::builder()
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let url = format!("{}/chat/completions", base.trim_end_matches('/'));
    let start = Instant::now();
    for attempt in 0..=3 {
        let mut req = http.post(&url).json(body);
        if !key.is_empty() {
            req = req.bearer_auth(key);
        }
        let resp = req.send().map_err(|e| {
            anyhow!(
                "POST /chat/completions: {}",
                mask_key(&crate::api::client::error_chain(&e), key)
            )
        })?;
        let status = resp.status().as_u16();
        let retry_after = resp
            .headers()
            .get("Retry-After")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        let bytes = resp.bytes()?;
        let raw = String::from_utf8_lossy(&bytes[..bytes.len().min(8 << 20)]).to_string();
        if !(200..300).contains(&status) {
            let error = ApiError {
                method: "POST".into(),
                path: "/chat/completions".into(),
                status,
                body: mask_key(&raw.chars().take(4096).collect::<String>(), key),
                retry_after: retry_after.clone(),
            };
            if status == 429 && attempt < 3 {
                thread::sleep(crate::api::client::retry_after_wait(&retry_after));
                continue;
            }
            return Err(error.into());
        }
        return response_chat(serde_json::from_str(&raw)?, start.elapsed().as_millis() as i64);
    }
    bail!("upstream chat retry limit exceeded")
}

fn mask_key(text: &str, key: &str) -> String {
    if key.is_empty() {
        text.into()
    } else {
        text.replace(key, "[redacted]")
    }
}

pub struct Runtime {
    pub name: String,
    pub host: String,
    pub notice: String,
    pub fallback: bool,
    pub extra_terms: Vec<DictionaryTerm>,
    pub ignored_spans: Cell<usize>,
    pub usage: RefCell<Usage>,
    model: String,
    base_url: String,
    key: String,
    timeout: Duration,
    mix_client: Option<RefCell<crate::api::client::Client>>,
}

impl Runtime {
    pub fn new(m: &ArgMatches, g: &Globals, name: &str) -> Result<Self> {
        let fallback = arg(m, "detector-fallback").unwrap_or("");
        if !fallback.is_empty() && fallback != "regex" {
            return Err(exit::usage("--detector-fallback must be regex"));
        }
        let detector_timeout = parse_duration(arg(m, "detector-timeout").unwrap_or("1m0s"))
            .map_err(|e| exit::usage(format!("invalid --detector-timeout: {e}")))?;
        let global_timeout = parse_duration(arg(m, "timeout").unwrap_or("1m0s"))
            .map_err(|e| exit::usage(format!("invalid --timeout: {e}")))?;
        let timeout = if detector_timeout.is_zero() {
            Duration::from_secs(60)
        } else {
            detector_timeout
        };
        let timeout = if !global_timeout.is_zero() {
            timeout.min(global_timeout)
        } else {
            timeout
        };
        let mut r = Self {
            name: name.into(),
            host: String::new(),
            notice: String::new(),
            fallback: fallback == "regex",
            extra_terms: Vec::new(),
            ignored_spans: Cell::new(0),
            usage: RefCell::new(Usage::default()),
            model: String::new(),
            base_url: String::new(),
            key: String::new(),
            timeout,
            mix_client: None,
        };
        if name == "regex" {
            return Ok(r);
        }
        let (kind, model) = name
            .split_once(':')
            .filter(|(_, model)| !model.is_empty())
            .ok_or_else(|| exit::usage(format!("invalid --detector {name:?}")))?;
        r.model = model.into();
        match kind {
            "local" => {
                let base = arg(m, "detector-base-url").unwrap_or("http://localhost:11434/v1");
                let url = Url::parse(base).map_err(|_| exit::usage("invalid --detector-base-url"))?;
                if !matches!(url.scheme(), "http" | "https")
                    || !matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "::1"))
                    || !url.username().is_empty()
                    || url.password().is_some()
                {
                    return Err(exit::usage("invalid --detector-base-url"));
                }
                r.host = url[url::Position::BeforeHost..url::Position::AfterPort].into();
                r.base_url = base.trim_end_matches('/').into();
                r.key = key_from_env(arg(m, "detector-key-env").unwrap_or(""), false)?;
            }
            "mixlayer" => {
                // A model detector sends the raw text for detection, so a
                // dry run must not use one (same rule as `shield ask`).
                if g.dry_run {
                    return Err(exit::usage(
                        "--dry-run requires --detector regex or local:<model> because a mixlayer detector sends the raw text over the network",
                    ));
                }
                // Bound each detector call by --detector-timeout (capped by
                // --timeout), as for the other detector runtimes.
                let client = commands::new_client_with_timeout(m, r.timeout)?;
                r.base_url = client.config.base_url.clone();
                let url = Url::parse(&r.base_url)?;
                r.host = url[url::Position::BeforeHost..url::Position::AfterPort].into();
                r.notice = MIXLAYER_NOTICE.into();
                if !g.json && !g.agent {
                    eprintln!("{}", r.notice);
                }
                if let Some(env) = arg(m, "detector-key-env").filter(|s| !s.is_empty()) {
                    r.key = key_from_env(env, false)?;
                } else {
                    r.mix_client = Some(RefCell::new(client));
                }
            }
            _ => return Err(exit::usage(format!("invalid --detector {name:?}"))),
        }
        Ok(r)
    }

    fn call(&self, prompt: &str) -> Result<String> {
        let (instruction, data) = prompt
            .split_once(" Text: ")
            .ok_or_else(|| anyhow!("invalid detector prompt"))?;
        let system = if self.name.starts_with("local:") {
            format!("/no_think\n{instruction}")
        } else {
            instruction.into()
        };
        let mut request = json!({"model": self.model, "messages": [{"role":"system","content":system},{"role":"user","content":format!("Text: {data}")}], "temperature":0.0, "reasoning_effort":"none", "response_format":{"type":"json_object"}});
        let mut response = self.chat(&request);
        if response
            .as_ref()
            .err()
            .and_then(|e| e.downcast_ref::<ApiError>())
            .is_some_and(|e| e.status == 400)
        {
            request.as_object_mut().unwrap().remove("response_format");
            response = self.chat(&request);
        }
        let response = response?;
        let mut usage = self.usage.borrow_mut();
        usage.prompt += response.usage["prompt_tokens"].as_i64().unwrap_or(0);
        usage.completion += response.usage["completion_tokens"].as_i64().unwrap_or(0);
        usage.cache += ["cache_read_input_tokens", "input_cache_read_tokens"]
            .iter()
            .filter_map(|key| response.usage[*key].as_i64())
            .chain(response.usage["prompt_tokens_details"]["cached_tokens"].as_i64())
            .max()
            .unwrap_or(0);
        usage.unknown |= response.usage["prompt_tokens"].is_null() || response.usage["completion_tokens"].is_null();
        Ok(response.answer)
    }

    fn chat(&self, body: &Value) -> Result<ChatResult> {
        if let Some(client) = &self.mix_client {
            let start = Instant::now();
            let value = client.borrow_mut().post("/chat/completions", body)?;
            response_chat(value, start.elapsed().as_millis() as i64)
        } else {
            external_chat(&self.base_url, &self.key, body, self.timeout)
        }
    }

    pub fn detect(&self, text: &str, terms: &[DictionaryTerm]) -> Result<(Vec<Entity>, bool)> {
        let mut combined = terms.to_vec();
        combined.extend(self.extra_terms.clone());
        let call = |prompt: &str| self.call(prompt);
        let (entities, fallback, ignored) = shield::detect_with_stats(
            text,
            &combined,
            (self.name != "regex").then_some(&call as &shield::ModelCall<'_>),
            self.fallback,
        )?;
        self.ignored_spans.set(self.ignored_spans.get() + ignored);
        Ok((entities, fallback))
    }

    pub fn redact(&self, store: &mut Store, text: &str, reveal: bool) -> Result<(RedactionResult, bool)> {
        let terms = store.dictionary_terms()?;
        let (entities, fallback) = self.detect(text, &terms)?;
        Ok((shield::redact_entities(store, text, entities, reveal)?, fallback))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ask_matches(args: &[&str]) -> clap::ArgMatches {
        let mut argv = vec!["mixlayer", "shield", "ask", "hi"];
        argv.extend_from_slice(args);
        let root = crate::cli::build().try_get_matches_from(argv).unwrap();
        root.subcommand().unwrap().1.subcommand().unwrap().1.clone()
    }

    #[test]
    fn go_upstream_presets_and_validation() {
        for (name, base, env, host) in [
            ("mixlayer", "https://mixlayer.ai/v1", "MIXLAYER_API_KEY", "mixlayer.ai"),
            (
                "openai",
                "https://api.openai.com/v1",
                "OPENAI_API_KEY",
                "api.openai.com",
            ),
            (
                "anthropic",
                "https://api.anthropic.com/v1",
                "ANTHROPIC_API_KEY",
                "api.anthropic.com",
            ),
            (
                "openrouter",
                "https://openrouter.ai/api/v1",
                "OPENROUTER_API_KEY",
                "openrouter.ai",
            ),
        ] {
            let resolved = resolve_upstream(&ask_matches(&["--upstream", name])).unwrap();
            assert_eq!(
                (
                    resolved.base_url.as_str(),
                    resolved.key_env.as_str(),
                    resolved.host.as_str()
                ),
                (base, env, host)
            );
        }
        assert!(resolve_upstream(&ask_matches(&["--upstream", "custom"])).is_err());
        assert!(
            resolve_upstream(&ask_matches(&[
                "--upstream",
                "custom",
                "--upstream-base-url",
                "https://user:secret@example.test/v1",
                "--upstream-key-env",
                "TEST_KEY"
            ]))
            .is_err()
        );
        assert!(
            resolve_upstream(&ask_matches(&[
                "--upstream",
                "custom",
                "--upstream-base-url",
                "https://example.test/v1?bad=1",
                "--upstream-key-env",
                "TEST_KEY"
            ]))
            .is_err()
        );
        assert!(key_from_env("bad-name", false).is_err());
        assert!(key_from_env("MIXLAYER_TEST_ABSENT_KEY_8D6A321C", false).is_err());
    }

    #[test]
    fn detector_local_url_is_loopback_only() {
        let m = ask_matches(&[
            "--detector",
            "local:fake",
            "--detector-base-url",
            "http://example.test/v1",
        ]);
        assert_eq!(
            crate::exit::code_of(&Runtime::new(&m, &Globals::default(), "local:fake").err().unwrap()),
            Code::Usage
        );
        let m = ask_matches(&[
            "--detector",
            "local:fake",
            "--detector-base-url",
            "http://127.0.0.1:11434/v1",
        ]);
        assert!(Runtime::new(&m, &Globals::default(), "local:fake").is_ok());
    }

    #[test]
    fn dry_run_refuses_mixlayer_detector() {
        let m = ask_matches(&["--detector", "mixlayer:some/model"]);
        let g = Globals {
            dry_run: true,
            ..Globals::default()
        };
        let err = Runtime::new(&m, &g, "mixlayer:some/model").err().expect("must refuse");
        assert_eq!(crate::exit::code_of(&err), Code::Usage);
    }
}
