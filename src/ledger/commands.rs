use std::time::Instant;

use clap::ArgMatches;
use regex::Regex;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::human;
use crate::Globals;
use crate::api::client::Client;
use crate::api::commands::{classified, models, new_client};
use crate::exit::{self, Code};
use crate::store::{RunRecord, Store};

const DEFAULT_RUNGS: &[&str] = &[
    "qwen/qwen3.5-4b-free",
    "qwen/qwen3.5-9b",
    "qwen/qwen3.6-35b-a3b",
    "qwen/qwen3.8-27b",
    "z-ai/glm-5.3",
];
fn arg<'a>(m: &'a ArgMatches, name: &str) -> Option<&'a str> {
    m.try_get_one::<String>(name).ok().flatten().map(String::as_str)
}
fn changed(m: &ArgMatches, name: &str) -> bool {
    m.value_source(name) == Some(clap::parser::ValueSource::CommandLine)
}
fn bool_arg(m: &ArgMatches, name: &str) -> bool {
    m.try_get_one::<bool>(name).ok().flatten().copied().unwrap_or(false)
}
fn words(m: &ArgMatches, name: &str) -> Vec<String> {
    crate::cli::positional_words(crate::spec_types::find(name).expect("command"), m)
}
fn prompt(m: &ArgMatches, name: &str) -> anyhow::Result<String> {
    words(m, name)
        .first()
        .cloned()
        .ok_or_else(|| exit::usage("question is required"))
}
fn print(value: &Value, g: &Globals) -> anyhow::Result<()> {
    crate::output::print_json(value, g)
}
fn output(value: &Value) -> anyhow::Result<()> {
    print(value, &Globals::default())
}
fn db(m: &ArgMatches) -> anyhow::Result<Store> {
    match arg(m, "db").filter(|s| !s.is_empty()) {
        Some(p) => Store::open(p),
        None => Store::open_default(),
    }
}
fn id(prefix: &str) -> String {
    let mut b = [0u8; 16];
    let _ = getrandom::fill(&mut b);
    format!("{prefix}_{}", hex::encode(&Sha256::digest(b)[..8]))
}
fn parse_i64(m: &ArgMatches, name: &str, fallback: i64) -> anyhow::Result<i64> {
    arg(m, name)
        .unwrap_or("")
        .parse::<i64>()
        .or_else(|_| {
            if arg(m, name).is_none() {
                Ok(fallback)
            } else {
                Err("invalid number")
            }
        })
        .map_err(|_| exit::usage(format!("invalid --{name}")))
}
fn parse_f64(m: &ArgMatches, name: &str, fallback: f64) -> anyhow::Result<f64> {
    let v = arg(m, name)
        .map_or(Ok(fallback), str::parse::<f64>)
        .map_err(|_| exit::usage(format!("invalid --{name}")))?;
    if !v.is_finite() {
        return Err(exit::usage(format!("invalid --{name}")));
    }
    Ok(v)
}
fn rungs(spec: &str) -> Vec<String> {
    if spec.trim().is_empty() || spec.eq_ignore_ascii_case("all") {
        return DEFAULT_RUNGS.iter().map(|s| (*s).into()).collect();
    }
    spec.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}
fn catalog(m: &ArgMatches) -> (Vec<Value>, String) {
    models::load_live_catalog(m).unwrap_or((Vec::new(), "offline_fallback".into()))
}
fn active_rungs(rows: &[Value]) -> Vec<String> {
    let mut ranked: Vec<_> = rows
        .iter()
        .filter_map(|r| {
            let id = r["model_sku"].as_str()?;
            if r["status"].as_str().unwrap_or("").eq_ignore_ascii_case("active")
                && !id.to_ascii_lowercase().contains("embed")
                && !r["model_type"]
                    .as_str()
                    .unwrap_or("")
                    .to_ascii_lowercase()
                    .contains("embed")
            {
                Some((
                    id.to_owned(),
                    r["price_m_output_tokens_usd"].as_f64().unwrap_or(f64::MAX),
                ))
            } else {
                None
            }
        })
        .collect();
    ranked.sort_by(|a, b| a.1.total_cmp(&b.1));
    ranked.into_iter().map(|p| p.0).collect()
}
fn catalog_rungs(spec: &str, rows: &[Value], source: &str) -> Vec<String> {
    if spec.trim().is_empty() || spec.eq_ignore_ascii_case("all") {
        let active = active_rungs(rows);
        if !active.is_empty() {
            return active;
        }
        if source == "ids_only" {
            return rows
                .iter()
                .filter_map(|r| r["model_sku"].as_str().map(str::to_owned))
                .collect();
        }
    }
    rungs(spec)
}
fn frontier(rows: &[Value], source: &str) -> String {
    models::default_model(rows, source)
}
fn new_chat_client(m: &ArgMatches) -> anyhow::Result<Client> {
    new_client(m)
}

#[derive(Clone, Default)]
pub(super) struct ResultRow {
    pub model: String,
    pub answer: String,
    pub reasoning: String,
    pub prompt_tokens: i64,
    pub cache_read_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
    pub usage_reported: bool,
    pub cost_usd: f64,
    pub cost_unknown: bool,
    pub latency_ms: i64,
    pub error: Option<String>,
    pub error_code: Option<Code>,
    pub final_error: Option<String>,
    pub checker: Option<String>,
    pub checker_score: Option<f64>,
    pub checker_model: Option<String>,
    pub checker_error: Option<String>,
    pub checker_usage: Option<Value>,
    pub raw_response: Option<Value>,
}
impl ResultRow {
    fn json(&self) -> Value {
        let mut v = json!({"model":self.model,"usage_reported":self.usage_reported,"cost_usd":if self.cost_unknown || self.error.is_some() {None} else {Some(self.cost_usd)}});
        for (key, value) in [("answer", &self.answer), ("reasoning", &self.reasoning)] {
            if !value.is_empty() {
                v[key] = value.as_str().into();
            }
        }
        for (key, value) in [
            ("prompt_tokens", self.prompt_tokens),
            ("cache_read_tokens", self.cache_read_tokens),
            ("completion_tokens", self.completion_tokens),
            ("total_tokens", self.total_tokens),
            ("latency_ms", self.latency_ms),
        ] {
            if value != 0 {
                v[key] = value.into();
            }
        }
        if let Some(e) = &self.error {
            v["error"] = e.as_str().into();
        }
        if let Some(checker) = &self.checker {
            v["checker"] = checker.as_str().into();
            v["checker_score"] = self.checker_score.into();
            v["checker_model"] = self.checker_model.as_deref().unwrap_or("").into();
        }
        if let Some(error) = &self.checker_error {
            v["checker_error"] = error.as_str().into();
        }
        if let Some(usage) = &self.checker_usage {
            v["checker_usage"] = usage.clone();
        }
        v
    }
}
fn chat(client: &mut Client, prompt: &str, model: &str, reasoning: bool, seed: i64) -> ResultRow {
    let mut result = ResultRow {
        model: model.into(),
        cost_unknown: true,
        ..Default::default()
    };
    let mut body = json!({"model":model,"messages":[{"role":"user","content":prompt}],"thinking":reasoning});
    if seed != 0 {
        body["seed"] = seed.into();
    }
    let start = Instant::now();
    match client.post("/chat/completions", &body) {
        Ok(response) => {
            result.raw_response = Some(response.clone());
            result.latency_ms = start.elapsed().as_millis() as i64;
            result.answer = response["choices"][0]["message"]["content"]
                .as_str()
                .unwrap_or("")
                .into();
            if result.answer.is_empty() {
                return ResultRow {
                    model: model.into(),
                    error: Some("chat response has no answer".into()),
                    error_code: Some(Code::Api),
                    cost_unknown: true,
                    ..Default::default()
                };
            }
            result.reasoning = response["choices"][0]["message"]["reasoning_content"]
                .as_str()
                .unwrap_or("")
                .into();
            let u = &response["usage"];
            result.usage_reported = u.get("prompt_tokens").is_some() && u.get("completion_tokens").is_some();
            result.prompt_tokens = u["prompt_tokens"].as_i64().unwrap_or(0);
            result.completion_tokens = u["completion_tokens"].as_i64().unwrap_or(0);
            result.total_tokens = u["total_tokens"].as_i64().unwrap_or(0);
            result.cache_read_tokens = [
                u["cache_read_input_tokens"].as_i64(),
                u["input_cache_read_tokens"].as_i64(),
                u["prompt_tokens_details"]["cached_tokens"].as_i64(),
            ]
            .into_iter()
            .flatten()
            .max()
            .unwrap_or(0)
            .max(0);
        }
        Err(e) => {
            result.error = Some(e.to_string());
            let final_error = classified(e, &Globals::default(), "ladder");
            result.error_code = Some(exit::code_of(&final_error));
            result.final_error = Some(final_error.to_string());
        }
    }
    result
}
fn across(
    m: &ArgMatches,
    prompt: &str,
    models: &[String],
    reasoning: bool,
    seed: i64,
) -> anyhow::Result<Vec<ResultRow>> {
    let template = new_chat_client(m)?;
    let cfg = template.config;
    let timeout = crate::api::client::parse_duration(arg(m, "timeout").unwrap_or("1m0s"))?;
    let rate = parse_f64(m, "rate-limit", 0.0)?;
    let no_cache = bool_arg(m, "no-cache");
    let default_models;
    let models = if models.is_empty() {
        default_models = rungs("all");
        &default_models
    } else {
        models
    };
    Ok(std::thread::scope(|scope| {
        let handles: Vec<_> = models
            .iter()
            .map(|model| {
                let cfg = cfg.clone();
                scope.spawn(move || match Client::new(cfg, timeout, rate, no_cache) {
                    Ok(mut client) => chat(&mut client, prompt, model, reasoning, seed),
                    Err(e) => ResultRow {
                        model: model.clone(),
                        error: Some(e.to_string()),
                        cost_unknown: true,
                        ..Default::default()
                    },
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| {
                h.join().unwrap_or_else(|_| ResultRow {
                    error: Some("model call panicked".into()),
                    cost_unknown: true,
                    ..Default::default()
                })
            })
            .collect()
    }))
}
fn apply_costs(results: &mut [ResultRow], rows: &[Value]) {
    for r in results {
        let free = rows.iter().any(|row| {
            row["model_sku"] == r.model
                && row["price_m_input_tokens_usd"].as_f64() == Some(0.0)
                && row["price_m_output_tokens_usd"].as_f64() == Some(0.0)
        });
        if r.error.is_none() && !r.answer.is_empty() && (r.usage_reported || free) {
            let usage = json!({"prompt_tokens":r.prompt_tokens,"completion_tokens":r.completion_tokens,"prompt_tokens_details":{"cached_tokens":r.cache_read_tokens}});
            if let Some(cost) = models::price(&r.model, rows, &usage) {
                r.cost_usd = cost;
                r.cost_unknown = false;
            }
        }
    }
}
fn first_confident(results: &[ResultRow]) -> &str {
    results
        .iter()
        .find(|r| r.error.is_none() && !r.answer.trim().is_empty())
        .map(|r| r.model.as_str())
        .unwrap_or("")
}
fn outcome(results: &[ResultRow]) -> anyhow::Result<()> {
    let failed: Vec<_> = results
        .iter()
        .filter(|r| r.error.is_some() || r.answer.is_empty())
        .collect();
    if failed.is_empty() {
        return Ok(());
    }
    if failed.len() == results.len() {
        return Err(exit::err(
            failed[0].error_code.unwrap_or(Code::Api),
            failed[0]
                .final_error
                .clone()
                .or_else(|| failed[0].error.clone())
                .unwrap_or_else(|| format!("model {} returned no answer", failed[0].model)),
        ));
    }
    Err(exit::err(
        Code::Partial,
        format!("{} of {} model calls failed", failed.len(), results.len()),
    ))
}
fn save_runs(
    s: &mut Store,
    command: &str,
    group: &str,
    prompt: &str,
    seed: i64,
    results: &[ResultRow],
) -> anyhow::Result<()> {
    for r in results {
        s.save_run(RunRecord {
            id: id("run"),
            group_id: group.into(),
            command: command.into(),
            prompt: prompt.into(),
            answer: r.answer.clone(),
            reasoning: r.reasoning.clone(),
            model: r.model.clone(),
            seed,
            raw_json: Some(r.json()),
            prompt_tokens: r.prompt_tokens,
            completion_tokens: r.completion_tokens,
            total_tokens: r.total_tokens,
            cost_usd: r.cost_usd,
            cost_unknown: r.cost_unknown || r.error.is_some(),
            latency_ms: r.latency_ms,
            ..Default::default()
        })?;
    }
    Ok(())
}
#[allow(clippy::too_many_arguments)]
fn save_group(
    s: &mut Store,
    command: &str,
    prompt: &str,
    rungs: &[String],
    first: &str,
    judge: &str,
    seed: i64,
    results: &[ResultRow],
) -> anyhow::Result<String> {
    let group = id("ladder");
    s.save_ladder(&group, prompt, rungs, first, judge)?;
    save_runs(s, command, &group, prompt, seed, results)?;
    Ok(group)
}
fn result_array(results: &[ResultRow]) -> Value {
    Value::Array(results.iter().map(ResultRow::json).collect())
}

pub fn ladder(m: &ArgMatches, g: &Globals) -> anyhow::Result<()> {
    let p = prompt(m, "ladder")?;
    let spec = arg(m, "rungs").unwrap_or("all");
    if g.dry_run {
        return print(
            &json!({"dry_run":true,"endpoint":"/chat/completions","models":rungs(spec),"prompt":p,"catalog_source":if spec=="all" {"offline_fallback"} else {"explicit"}}),
            g,
        );
    }
    let seed = parse_i64(m, "seed", 0)?;
    let mut s = db(m)?;
    let (rows, source) = catalog(m);
    let models = catalog_rungs(spec, &rows, &source);
    let mut results = across(m, &p, &models, bool_arg(m, "reasoning"), seed)?;
    apply_costs(&mut results, &rows);
    let first = first_confident(&results).to_owned();
    save_group(&mut s, "ladder", &p, &models, &first, "", seed, &results)?;
    if human::want(g) {
        human::ladder(&results, &first, g);
    } else {
        output(&json!({"results":result_array(&results),"first_confident_model":first,"catalog_source":source}))?;
    }
    outcome(&results)
}
fn confidence(answer: &str) -> f64 {
    if answer.trim().is_empty() {
        return 0.0;
    }
    let lower = answer.to_ascii_lowercase();
    if ["not sure", "uncertain", "can't determine", "cannot determine"]
        .iter()
        .any(|s| lower.contains(s))
    {
        return 0.4;
    }
    let re = Regex::new(r"(?i)\bconfidence\s*[:=]\s*\**\s*(high|medium|low)\b").expect("regex");
    match re.captures_iter(answer).last().map(|c| c[1].to_ascii_lowercase()) {
        Some(v) if v == "high" => 0.9,
        Some(v) if v == "medium" => 0.7,
        Some(_) => 0.4,
        None => 0.65,
    }
}
pub fn escalate(m: &ArgMatches, g: &Globals) -> anyhow::Result<()> {
    let p = prompt(m, "escalate")?;
    let spec = arg(m, "rungs").unwrap_or("all");
    let checker = arg(m, "checker").unwrap_or("heuristic");
    if checker != "heuristic" && checker != "jini" {
        return Err(exit::usage("--checker must be heuristic or jini"));
    }
    let threshold = parse_f64(m, "threshold", 0.8)?;
    if !(0.0..=1.0).contains(&threshold) {
        return Err(exit::usage("--threshold must be between 0 and 1"));
    }
    let target = parse_f64(m, "confidence", 0.85)?;
    let checker_model = if checker == "jini" {
        Some(super::decision::model(m, "checker-model")?)
    } else {
        None
    };
    if let Some(model) = &checker_model {
        super::decision::validate_checker(model, &p)?;
    }
    if g.dry_run {
        let mut v = json!({"dry_run":true,"endpoint":"/chat/completions","models":rungs(spec),"prompt":p,"catalog_source":if spec=="all" {"offline_fallback"} else {"explicit"},"checker":checker});
        if let Some(model) = checker_model {
            v["checker_url"] = super::decision::target_url(m)?.into();
            v["checker_body_template"] = super::decision::checker_request(&model, &p, "<rung response>");
            v["threshold"] = threshold.into();
        }
        return print(&v, g);
    }
    let mut s = db(m)?;
    let (rows, source) = catalog(m);
    let configured = catalog_rungs(spec, &rows, &source);
    let mut results = Vec::new();
    let mut checker_error = None;
    for rung in &configured {
        let request_prompt = if checker == "heuristic" {
            format!(
                "{p}\n\nReturn a concise answer. End with exactly one line: CONFIDENCE: high, medium, or low. Use low or medium when uncertain."
            )
        } else {
            p.clone()
        };
        let mut part = across(m, &request_prompt, std::slice::from_ref(rung), false, 0)?;
        apply_costs(&mut part, &rows);
        let mut r = part.remove(0);
        r.checker = Some(checker.into());
        r.checker_model = Some(checker_model.clone().unwrap_or_default());
        let mut stop = false;
        if r.error.is_none() && !r.answer.is_empty() {
            if checker == "heuristic" {
                let score = confidence(&r.answer);
                r.checker_score = Some(score);
                stop = score >= target;
            } else if let Some(model) = &checker_model {
                match super::decision::request(m, &super::decision::checker_request(model, &p, &r.answer)) {
                    Ok(response) => {
                        r.checker_score = response["answers"]["correct"]["noul"].as_f64();
                        r.checker_usage = response.get("usage").cloned();
                        stop = r.checker_score.is_some_and(|n| n >= threshold);
                    }
                    Err(e) => {
                        let message = format!("jini checker failed after rung {rung}: {e}");
                        r.checker_error = Some(message.clone());
                        checker_error = Some(exit::err(exit::code_of(&e), message));
                    }
                }
            }
        }
        results.push(r);
        if stop || checker_error.is_some() {
            break;
        }
    }
    let total: f64 = results.iter().map(|r| r.cost_usd).sum();
    let unknown = checker == "jini" || results.iter().any(|r| r.cost_unknown);
    let tried = configured.into_iter().take(results.len()).collect::<Vec<_>>();
    let last = results.last().map(|r| r.model.clone()).unwrap_or_default();
    let group = save_group(&mut s, "escalate", &p, &tried, &last, "", 0, &results)?;
    let mut baseline = None;
    let mut reason = String::new();
    let first = first_confident(&results);
    let frontier = frontier(&rows, &source);
    if first.is_empty() {
        reason = "no rung returned an answer".into();
    } else if frontier.is_empty()
        || rows
            .iter()
            .all(|r| r["model_sku"] != frontier || !r["price_m_input_tokens_usd"].is_number())
    {
        reason = "frontier model is unpriced".into();
    } else if results.iter().any(|r| r.model == frontier && r.error.is_some()) {
        reason = "frontier baseline model failed".into();
    } else {
        baseline = models::price(
            &frontier,
            &rows,
            &json!({"prompt_tokens":1000,"completion_tokens":1000}),
        );
        if baseline.is_none() {
            reason = "frontier model is unpriced".into();
        }
    }
    let cost = (!unknown).then_some(human::round(total));
    let saved = baseline.zip(cost).map(|(b, c)| human::round(b - c));
    if human::want(g) {
        human::escalate(&results, cost, baseline, saved, &reason, g);
    } else {
        output(
            &json!({"group_id":group,"results":result_array(&results),"cost_usd":cost,"cost_reason":if checker=="jini" {"classification checker cost is not included in the generation catalog"} else {""},"frontier_baseline_usd":baseline,"saved_usd":saved,"baseline_reason":reason,"catalog_source":source}),
        )?;
    }
    if let Some(e) = checker_error {
        return Err(e);
    }
    outcome(&results)
}
fn council_prompt(question: &str, results: &[ResultRow]) -> String {
    let mut p = format!("Question: {question}\n\nSynthesize these model answers and flag disagreements:\n");
    for (i, r) in results.iter().enumerate() {
        // Go's struct serializer uses model, reasoning, answer field order.
        let encoded = if r.reasoning.is_empty() {
            format!("{{\"model\":{},\"answer\":{}}}", json!(r.model), json!(r.answer))
        } else {
            format!(
                "{{\"model\":{},\"reasoning\":{},\"answer\":{}}}",
                json!(r.model),
                json!(r.reasoning),
                json!(r.answer)
            )
        };
        p.push_str(&format!(
            "\n---BEGIN MEMBER {}---\n{}\n---END MEMBER {}---\n",
            i + 1,
            encoded,
            i + 1
        ));
    }
    p
}
pub fn council(m: &ArgMatches, g: &Globals) -> anyhow::Result<()> {
    let p = prompt(m, "council")?;
    let spec = arg(m, "members").unwrap_or("all");
    let default_judge = arg(m, "judge").unwrap_or("z-ai/glm-5.3");
    if g.dry_run {
        let mut all = rungs(spec);
        all.push(default_judge.into());
        return print(
            &json!({"dry_run":true,"endpoint":"/chat/completions","models":all,"prompt":p,"catalog_source":"offline_fallback"}),
            g,
        );
    }
    let mut s = db(m)?;
    let (rows, source) = catalog(m);
    let members = catalog_rungs(spec, &rows, &source);
    let judge = if changed(m, "judge") {
        default_judge.into()
    } else {
        frontier(&rows, &source)
    };
    let mut results = across(m, &p, &members, true, 0)?;
    apply_costs(&mut results, &rows);
    let judge_prompt = council_prompt(&p, &results);
    let mut judged = across(m, &judge_prompt, std::slice::from_ref(&judge), false, 0)?;
    apply_costs(&mut judged, &rows);
    let mut all = members;
    all.push(judge.clone());
    let group = id("ladder");
    s.save_ladder(&group, &p, &all, "", &judge)?;
    save_runs(&mut s, "council", &group, &p, 0, &results)?;
    save_runs(&mut s, "council-judge", &group, &judge_prompt, 0, &judged)?;
    if human::want(g) {
        human::council(&results, &judged, g);
    } else {
        output(
            &json!({"group_id":group,"members":result_array(&results),"judge":result_array(&judged),"catalog_source":source}),
        )?;
    }
    results.extend(judged);
    outcome(&results)
}
fn run_json(r: &RunRecord) -> Value {
    let created = r
        .created_at
        .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        .unwrap_or_else(|| "0001-01-01T00:00:00Z".into());
    let mut v = json!({"id":r.id,"command":r.command,"prompt":r.prompt,"cost_usd":if r.cost_unknown {None} else {Some(r.cost_usd)},"created_at":created});
    if !r.group_id.is_empty() {
        v["group_id"] = r.group_id.as_str().into();
    }
    if !r.reasoning.is_empty() {
        v["reasoning"] = r.reasoning.as_str().into();
    }
    if !r.answer.is_empty() {
        v["answer"] = r.answer.as_str().into();
    }
    if !r.model.is_empty() {
        v["model"] = r.model.as_str().into();
    }
    if r.seed != 0 {
        v["seed"] = r.seed.into();
    }
    if let Some(p) = &r.params_json {
        v["params_json"] = p.clone();
    }
    if let Some(raw) = &r.raw_json {
        v["raw_json"] = raw.clone();
    }
    for (key, value) in [
        ("prompt_tokens", r.prompt_tokens),
        ("completion_tokens", r.completion_tokens),
        ("total_tokens", r.total_tokens),
        ("latency_ms", r.latency_ms),
    ] {
        if value != 0 {
            v[key] = value.into();
        }
    }
    if r.cost_unknown {
        v["cost_unknown"] = true.into();
    }
    if !r.catalog_source.is_empty() {
        v["catalog_source"] = r.catalog_source.as_str().into();
    }
    v
}
#[allow(clippy::too_many_arguments)]
fn chat_save(
    m: &ArgMatches,
    g: &Globals,
    s: &mut Store,
    command: &str,
    prompt: &str,
    model: &str,
    seed: i64,
    thinking: bool,
    rows: &[Value],
    source: &str,
) -> anyhow::Result<RunRecord> {
    let mut client = new_chat_client(m)?;
    let r = chat(&mut client, prompt, model, thinking, seed);
    if let Some(e) = r.error {
        return Err(exit::err(r.error_code.unwrap_or(Code::Api), r.final_error.unwrap_or(e)));
    }
    if r.answer.is_empty() {
        return Err(exit::err(Code::Api, "chat response has no answer"));
    }
    let mut result = r.clone();
    apply_costs(std::slice::from_mut(&mut result), rows);
    let run = RunRecord {
        id: id("run"),
        command: command.into(),
        prompt: prompt.into(),
        answer: result.answer,
        reasoning: result.reasoning,
        model: model.into(),
        seed,
        params_json: Some(json!({"model":model,"seed":seed,"reasoning":thinking})),
        raw_json: r.raw_response.clone(),
        prompt_tokens: r.prompt_tokens,
        completion_tokens: r.completion_tokens,
        total_tokens: r.total_tokens,
        cost_usd: result.cost_usd,
        cost_unknown: result.cost_unknown,
        catalog_source: source.into(),
        latency_ms: r.latency_ms,
        ..Default::default()
    };
    s.save_run(run.clone()).map_err(|e| classified(e, g, command))?;
    Ok(run)
}
pub fn compare(m: &ArgMatches, g: &Globals) -> anyhow::Result<()> {
    let p = prompt(m, "compare")?;
    let mut cheap = arg(m, "cheap").unwrap_or("qwen/qwen3.5-9b").to_owned();
    let mut top = arg(m, "frontier").unwrap_or("z-ai/glm-5.3").to_owned();
    if g.dry_run {
        return print(
            &json!({"dry_run":true,"endpoint":"/chat/completions","models":[cheap,top],"prompt":p,"catalog_source":if changed(m,"cheap")&&changed(m,"frontier"){"explicit"}else{"offline_fallback"}}),
            g,
        );
    }
    let mut s = db(m)?;
    let (rows, source) = catalog(m);
    if !changed(m, "cheap")
        && let Some(first) = active_rungs(&rows).first()
    {
        cheap = first.clone();
    }
    if !changed(m, "frontier") {
        top = frontier(&rows, &source);
    }
    let a = chat_save(m, g, &mut s, "compare", &p, &cheap, 0, false, &rows, &source)?;
    let b = chat_save(m, g, &mut s, "compare", &p, &top, 0, false, &rows, &source)?;
    let history = s.search_runs(&p, 50).unwrap_or_default();
    let note = if history.len() > 2 {
        format!(
            "Evidence note: {} related saved runs exist in your local ledger for this prompt pattern.",
            history.len()
        )
    } else {
        "No prior ladder history found for an evidence-grounded quality rate.".into()
    };
    let delta = if a.cost_unknown || b.cost_unknown {
        None
    } else {
        Some(human::round(b.cost_usd - a.cost_usd))
    };
    if human::want(g) {
        let row = |r: &RunRecord| ResultRow {
            model: r.model.clone(),
            answer: r.answer.clone(),
            total_tokens: r.total_tokens,
            latency_ms: r.latency_ms,
            cost_usd: r.cost_usd,
            cost_unknown: r.cost_unknown,
            ..Default::default()
        };
        human::compare(&row(&a), &row(&b), delta, &note, g);
        Ok(())
    } else {
        output(&json!({"cheap":run_json(&a),"frontier":run_json(&b),"delta_usd":delta,"quality_note":note}))
    }
}
fn baseline(vs: &str, tokens: i64) -> f64 {
    let (i, o) = match vs.trim() {
        "claude-frontier" => (3.0, 15.0),
        _ => (5.0, 15.0),
    };
    human::round((tokens / 2) as f64 * i / 1e6 + (tokens - tokens / 2) as f64 * o / 1e6)
}
pub fn savings(m: &ArgMatches, g: &Globals) -> anyhow::Result<()> {
    let s = db(m)?;
    let runs = s.recent_runs(10000)?;
    let vs = arg(m, "vs").unwrap_or("gpt-frontier");
    let mut actual = 0.0;
    let mut base = 0.0;
    let mut tokens = 0;
    let mut unknown = 0;
    for r in &runs {
        if r.cost_unknown {
            unknown += 1;
            continue;
        }
        actual += r.cost_usd;
        tokens += r.total_tokens;
        base += baseline(vs, r.total_tokens);
    }
    if human::want(g) {
        human::savings(runs.len() - unknown, unknown, tokens, actual, base, vs, g);
        Ok(())
    } else {
        output(
            &json!({"runs":runs.len()-unknown,"excluded_unknown_cost_runs":unknown,"tokens":tokens,"actual_usd":human::round(actual),"baseline_usd":human::round(base),"saved_usd":human::round(base-actual),"baseline":vs}),
        )
    }
}
pub fn grep(m: &ArgMatches, g: &Globals) -> anyhow::Result<()> {
    let query = words(m, "grep")
        .first()
        .cloned()
        .ok_or_else(|| exit::usage("pattern is required"))?;
    let limit = parse_i64(m, "limit", 20)?;
    let s = db(m)?;
    let runs = s.search_runs(&query, limit)?;
    let _ = g;
    output(&Value::Array(runs.iter().map(run_json).collect()))
}
pub fn replay(m: &ArgMatches, g: &Globals) -> anyhow::Result<()> {
    let run_id = words(m, "replay")
        .first()
        .cloned()
        .ok_or_else(|| exit::usage("run-id is required"))?;
    let mut s = db(m)?;
    let original = s
        .get_run(&run_id)?
        .ok_or_else(|| exit::err(Code::Other, format!("run {run_id} not found")))?;
    if g.dry_run {
        return print(
            &json!({"dry_run":true,"endpoint":"/chat/completions","models":[original.model],"prompt_summary":{"bytes":original.prompt.len(),"sha256":hex::encode(Sha256::digest(original.prompt.as_bytes()))},"seed":original.seed}),
            g,
        );
    }
    let (rows, source) = catalog(m);
    let new = chat_save(
        m,
        g,
        &mut s,
        "replay",
        &original.prompt,
        &original.model,
        original.seed,
        !original.reasoning.is_empty(),
        &rows,
        &source,
    )?;
    output(&json!({"original":run_json(&original),"replay":run_json(&new),"same_answer":original.answer==new.answer}))
}
pub fn sql(m: &ArgMatches, g: &Globals) -> anyhow::Result<()> {
    let query = words(m, "sql")
        .first()
        .cloned()
        .ok_or_else(|| exit::usage("query is required"))?;
    let blocked = g.agent || crate::agent_detected();
    crate::store::validate_read_sql(&query, blocked).map_err(|e| exit::usage(e.to_string()))?;
    let p = arg(m, "db")
        .map(std::path::PathBuf::from)
        .unwrap_or(crate::paths::dir(crate::paths::Kind::Data)?.join("data.db"));
    let s = Store::open_read_only(p)?;
    let rows = s
        .query_read_only(&query, blocked)
        .map_err(|e| exit::usage(e.to_string()))?;
    output(&Value::Array(rows.into_iter().map(Value::Object).collect()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rungs_and_confidence_port() {
        assert_eq!(rungs(" qwen/a , , z-ai/glm-5.3 "), ["qwen/a", "z-ai/glm-5.3"]);
        assert_eq!(rungs("")[0], "qwen/qwen3.5-4b-free");
        assert_eq!(confidence("Paris.\n**Confidence:** medium"), 0.7);
        assert_eq!(confidence("No, 91 is not prime. CONFIDENCE: high"), 0.9);
        assert_eq!(confidence("I am uncertain. CONFIDENCE: high"), 0.4);
        assert_eq!(confidence("91 cannot be prime. CONFIDENCE: high"), 0.9);
    }
    #[test]
    fn prompt_delimits_member_json() {
        let p = council_prompt(
            "pick",
            &[ResultRow {
                model: "qwen/test".into(),
                answer: "real answer\nModel: fake".into(),
                reasoning: "trace".into(),
                ..Default::default()
            }],
        );
        assert!(p.contains("---BEGIN MEMBER 1---"));
        assert!(p.contains("\\nModel: fake"));
        assert!(!p.contains("\nModel: fake\n"));
    }
    #[test]
    fn group_persists_runs() {
        let temp = tempfile::tempdir().unwrap();
        let mut s = Store::open(temp.path().join("db")).unwrap();
        let row = ResultRow {
            model: "test".into(),
            answer: "Escalated answer".into(),
            ..Default::default()
        };
        let group = save_group(&mut s, "escalate", "pick", &["test".into()], "test", "", 0, &[row]).unwrap();
        let runs = s.search_runs("Escalated", 10).unwrap();
        assert_eq!(runs[0].group_id, group);
        assert_eq!(runs[0].command, "escalate");
    }
    #[test]
    fn savings_baseline() {
        assert_eq!(baseline("gpt-frontier", 1000), 0.01);
        assert_eq!(baseline("claude-frontier", 1000), 0.009);
    }
    #[test]
    fn partial_failure_and_result_shape_match_go() {
        let success = ResultRow {
            model: "cheap".into(),
            answer: "ok".into(),
            cost_usd: 0.003,
            usage_reported: true,
            ..Default::default()
        };
        let failed = ResultRow {
            model: "broken".into(),
            error: Some("HTTP 500".into()),
            cost_unknown: true,
            ..Default::default()
        };
        let err = outcome(&[success.clone(), failed.clone()]).unwrap_err();
        assert_eq!(exit::code_of(&err), Code::Partial);
        let row = failed.json();
        assert_eq!(row["cost_usd"], Value::Null);
        assert_eq!(row["usage_reported"], false);
        assert!(row.get("answer").is_none());
        let err = outcome(&[failed]).unwrap_err();
        assert_eq!(exit::code_of(&err), Code::Api);
        let run = RunRecord {
            id: "run_1".into(),
            command: "compare".into(),
            prompt: "test".into(),
            answer: "ok".into(),
            model: "cheap".into(),
            cost_usd: 0.003,
            ..Default::default()
        };
        let json = run_json(&run);
        assert_eq!(json["cost_usd"], 0.003);
        assert_eq!(json["created_at"], "0001-01-01T00:00:00Z");
        assert!(json.get("cost_unknown").is_none());
        assert!(json.get("seed").is_none());
    }
    #[test]
    fn ids_only_catalog_uses_live_ids() {
        let rows = vec![
            json!({"model_sku":"one","status":"unknown"}),
            json!({"model_sku":"two","status":"unknown"}),
        ];
        assert_eq!(catalog_rungs("all", &rows, "ids_only"), ["one", "two"]);
    }
}
