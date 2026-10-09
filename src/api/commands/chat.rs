use std::time::Instant;

use clap::ArgMatches;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::{arg, as_results, bool_arg, classified, models, new_client, print, words};
use crate::Globals;
use crate::exit::{self, Code};

fn optional_number(
    body: &mut serde_json::Map<String, Value>,
    m: &ArgMatches,
    flag: &str,
    key: &str,
) -> anyhow::Result<()> {
    if let Some(raw) = arg(m, flag).filter(|s| *s != "0" && *s != "0.0") {
        let number: Value = serde_json::from_str(raw).map_err(|_| exit::usage(format!("invalid --{flag}: {raw}")))?;
        if !number.is_number() {
            return Err(exit::usage(format!("invalid --{flag}: {raw}")));
        }
        body.insert(key.into(), number);
    }
    Ok(())
}

pub fn chat(m: &ArgMatches, g: &Globals) -> anyhow::Result<()> {
    if !g.dry_run && arg(m, "model").is_none() {
        return Err(exit::usage("required flag \"model\" not set"));
    }
    if !g.dry_run && arg(m, "messages").is_none() {
        return Err(exit::usage("required flag \"messages\" not set"));
    }
    let mut body = serde_json::Map::new();
    if let Some(model) = arg(m, "model") {
        body.insert("model".into(), model.into());
    }
    for (flag, key) in [
        ("messages", "messages"),
        ("tools", "tools"),
        ("response-format", "response_format"),
    ] {
        if let Some(raw) = arg(m, flag).filter(|s| !s.is_empty()) {
            let value = serde_json::from_str(raw).map_err(|e| exit::usage(format!("parsing --{flag} JSON: {e}")))?;
            body.insert(key.into(), value);
        }
    }
    for (flag, key) in [("tool-choice", "tool_choice"), ("reasoning-effort", "reasoning_effort")] {
        if let Some(value) = arg(m, flag).filter(|s| !s.is_empty()) {
            body.insert(key.into(), value.into());
        }
    }
    for (flag, key) in [
        ("temperature", "temperature"),
        ("top-p", "top_p"),
        ("top-k", "top_k"),
        ("frequency-penalty", "frequency_penalty"),
        ("presence-penalty", "presence_penalty"),
        ("repetition-penalty", "repetition_penalty"),
        ("max-completion-tokens", "max_completion_tokens"),
        ("max-tokens", "max_tokens"),
        ("seed", "seed"),
    ] {
        optional_number(&mut body, m, flag, key)?;
    }
    if let Ok(Some(stops)) = m.try_get_many::<String>("stop") {
        body.insert("stop".into(), Value::Array(stops.cloned().map(Value::String).collect()));
    }
    // Send an explicit `--thinking=false` / `--stream=false`: some models
    // think by default, and omitting the field would keep billing for it.
    for flag in ["stream", "thinking"] {
        if m.value_source(flag) == Some(clap::parser::ValueSource::CommandLine) {
            body.insert(flag.into(), bool_arg(m, flag).into());
        }
    }
    if g.dry_run {
        return print(&as_results(json!({"dry_run": true}), g), &Globals::default());
    }
    let mut client = new_client(m)?;
    let response = client
        .post("/chat/completions", &Value::Object(body))
        .map_err(|e| classified(e, g, "chat"))?;
    if !bool_arg(m, "stream") && response["choices"].as_array().is_none_or(Vec::is_empty) {
        return Err(exit::err(Code::Api, "malformed chat response body"));
    }
    if g.json {
        print(&as_results(response, g), &Globals::default())
    } else {
        println!("{response}");
        Ok(())
    }
}

fn run_id() -> String {
    let mut entropy = [0_u8; 16];
    let _ = getrandom::fill(&mut entropy);
    let hash = Sha256::digest(entropy);
    format!("run_{}", hex::encode(&hash[..8]))
}

pub fn ask(m: &ArgMatches, g: &Globals) -> anyhow::Result<()> {
    let args = words(m, "ask");
    let Some(prompt) = args.first() else {
        return Err(exit::usage("question is required"));
    };
    let explicit = arg(m, "model").filter(|s| !s.is_empty());
    if g.dry_run {
        let model = explicit.unwrap_or("z-ai/glm-5.3");
        return print(
            &json!({"dry_run": true, "endpoint": "/chat/completions", "models": [model], "messages": [{"role": "user", "content": prompt}], "catalog_source": if explicit.is_some() { "explicit" } else { "offline_fallback" }}),
            g,
        );
    }
    let seed = arg(m, "seed")
        .unwrap_or("0")
        .parse::<i64>()
        .map_err(|e| exit::usage(format!("invalid --seed: {e}")))?;
    let thinking = bool_arg(m, "show-thinking");
    let mut catalog = if explicit.is_none() {
        models::load_live_catalog(m).ok()
    } else {
        None
    };
    let model = explicit.map(str::to_owned).unwrap_or_else(|| {
        catalog
            .as_ref()
            .map(|(rows, source)| models::default_model(rows, source))
            .unwrap_or_else(|| "z-ai/glm-5.3".into())
    });
    let mut body = json!({"model": model, "messages": [{"role": "user", "content": prompt}], "thinking": thinking});
    if seed != 0 {
        body["seed"] = seed.into();
    }
    let mut client = new_client(m)?;
    let start = Instant::now();
    let response = client
        .post("/chat/completions", &body)
        .map_err(|e| classified(e, g, "ask"))?;
    let latency_ms = start.elapsed().as_millis() as u64;
    let answer = response["choices"][0]["message"]["content"].as_str().unwrap_or("");
    if answer.is_empty() {
        return Err(exit::err(Code::Api, "chat response has no answer"));
    }
    let reasoning = response["choices"][0]["message"]["reasoning_content"]
        .as_str()
        .unwrap_or("");
    let usage = &response["usage"];
    if catalog.is_none() {
        catalog = models::load_live_catalog(m).ok();
    }
    let source = catalog
        .as_ref()
        .map(|(_, source)| source.as_str())
        .unwrap_or("offline_fallback");
    let cost = catalog
        .as_ref()
        .and_then(|(rows, _)| models::price(&model, rows, usage));
    let params = json!({"model": model, "seed": seed, "reasoning": thinking});
    let mut run = json!({
        "id": run_id(), "command": "ask", "prompt": prompt, "answer": answer,
        "model": model, "params_json": params, "raw_json": response,
        "cost_usd": cost, "catalog_source": source,
        "latency_ms": latency_ms,
        "created_at": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Nanos, true),
    });
    if cost.is_none() {
        run["cost_unknown"] = true.into();
    }
    if !reasoning.is_empty() {
        run["reasoning"] = reasoning.into();
    }
    if seed != 0 {
        run["seed"] = seed.into();
    }
    for key in ["prompt_tokens", "completion_tokens", "total_tokens"] {
        if let Some(n) = usage[key].as_i64().filter(|n| *n != 0) {
            run[key] = n.into();
        }
    }
    let mut store = match m.try_get_one::<String>("db").ok().flatten().filter(|p| !p.is_empty()) {
        Some(p) => crate::store::Store::open(p)?,
        None => crate::store::Store::open_default()?,
    };
    store.save_run(crate::store::RunRecord {
        id: run["id"].as_str().unwrap_or_default().to_string(),
        command: "ask".into(),
        prompt: prompt.to_string(),
        answer: answer.to_string(),
        reasoning: reasoning.to_string(),
        model: model.clone(),
        seed,
        params_json: Some(params.clone()),
        raw_json: Some(response.clone()),
        prompt_tokens: usage["prompt_tokens"].as_i64().unwrap_or(0),
        completion_tokens: usage["completion_tokens"].as_i64().unwrap_or(0),
        total_tokens: usage["total_tokens"].as_i64().unwrap_or(0),
        cost_usd: cost.unwrap_or(0.0),
        cost_unknown: cost.is_none(),
        catalog_source: source.to_string(),
        latency_ms: latency_ms as i64,
        ..Default::default()
    })?;
    if g.json {
        print(&run, &Globals::default())
    } else {
        if thinking && !reasoning.is_empty() {
            println!("{reasoning}\n");
        }
        println!("{answer}");
        Ok(())
    }
}
