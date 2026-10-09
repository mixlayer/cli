// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

mod runtime;

use std::collections::{BTreeMap, HashMap};
use std::fs::{self, OpenOptions};
use std::io::{IsTerminal, Read};
use std::path::Path;
use std::time::Instant;

use anyhow::{Result, anyhow};
use clap::ArgMatches;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::Globals;
use crate::api::commands::{self, models};
use crate::exit::{self, Code};
use crate::output;
use crate::shield::{self, DictionaryTerm, Entity, EvalCounts, EvalMetrics, EvalResult, EvalSample, RedactionResult};
use crate::store::{AuditRecord, RunRecord, Store};
use runtime::{PLACEHOLDER_INSTRUCTION, Runtime, Upstream, external_chat, key_from_env, resolve_upstream};

fn arg<'a>(m: &'a ArgMatches, name: &str) -> Option<&'a str> {
    m.try_get_one::<String>(name).ok().flatten().map(String::as_str)
}
fn bool_arg(m: &ArgMatches, name: &str) -> bool {
    m.try_get_one::<bool>(name).ok().flatten().copied().unwrap_or(false)
}
fn strings(m: &ArgMatches, name: &str) -> Vec<String> {
    m.try_get_many::<String>(name)
        .ok()
        .flatten()
        .map(|v| v.cloned().collect())
        .unwrap_or_default()
}
fn words(m: &ArgMatches, command: &str) -> Vec<String> {
    crate::cli::positional_words(crate::spec_types::find(command).expect("command in spec"), m)
}
fn store(m: &ArgMatches) -> Result<Store> {
    match arg(m, "db").filter(|s| !s.is_empty()) {
        Some(path) => Store::open(path),
        None => Store::open_default(),
    }
}
fn json_out(v: &Value, g: &Globals) -> Result<()> {
    output::print_json(v, g)
}
fn raw_json_out(v: &Value) -> Result<()> {
    output::print_json(v, &Globals::default())
}
fn text_out(text: &str) -> Result<()> {
    output::write_stdout(text.as_bytes())
}
fn color_enabled(g: &Globals) -> bool {
    !g.no_color
        && !g.agent
        && !crate::agent_detected()
        && std::env::var_os("NO_COLOR").is_none()
        && std::env::var("TERM").as_deref() != Ok("dumb")
        && std::io::stdout().is_terminal()
}
fn color(text: &str, code: u8, g: &Globals) -> String {
    if color_enabled(g) {
        format!("\x1b[{code}m{text}\x1b[0m")
    } else {
        text.into()
    }
}
fn file(path: &str) -> Result<String> {
    if path == "-" {
        let mut s = String::new();
        std::io::stdin().read_to_string(&mut s)?;
        Ok(s)
    } else {
        Ok(fs::read_to_string(path)?)
    }
}
fn required_file(path: &str) -> Result<String> {
    Ok(fs::read_to_string(path)?)
}
fn text_or_file(input: &str) -> Result<String> {
    if input == "-" || Path::new(input).is_file() {
        file(input)
    } else {
        Ok(input.into())
    }
}
fn write_private(path: &str, bytes: &[u8]) -> Result<()> {
    let mut opts = OpenOptions::new();
    opts.create(true).truncate(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    use std::io::Write;
    let mut f = opts.open(path)?;
    f.write_all(bytes)?;
    Ok(())
}
fn required(m: &ArgMatches, command: &str) -> Result<String> {
    words(m, command)
        .first()
        .cloned()
        .ok_or_else(|| exit::usage(format!("{command} requires an argument")))
}
fn id(prefix: &str) -> Result<String> {
    let mut entropy = [0_u8; 16];
    getrandom::fill(&mut entropy).map_err(|e| anyhow!("ID entropy: {e}"))?;
    Ok(format!("{prefix}_{}", hex::encode(&Sha256::digest(entropy)[..8])))
}
fn columns(m: &ArgMatches, data: &str) -> Result<Vec<DictionaryTerm>> {
    let cols = strings(m, "mask-columns");
    shield::csv_column_terms(data, &cols.iter().map(String::as_str).collect::<Vec<_>>())
        .map_err(|e| exit::usage(e.to_string()))
}
fn entity_json(e: &Entity) -> Value {
    json!({"kind":e.kind,"value":e.value,"start":e.start,"end":e.end,"risk":e.risk})
}
fn redaction_json(r: &RedactionResult) -> Value {
    json!({"text":r.text,"entities":r.entities.iter().map(|e| {
        let mut value = json!({"kind":e.kind,"token":e.token,"start":e.start,"end":e.end,"risk":e.risk});
        if !e.value.is_empty() { value["value"] = e.value.clone().into(); }
        value
    }).collect::<Vec<_>>(),"risk":r.risk})
}
fn classify(err: anyhow::Error, g: &Globals) -> anyhow::Error {
    if exit::code_of(&err) != Code::Other {
        err
    } else if err.downcast_ref::<crate::api::client::ApiError>().is_some() {
        commands::classified(err, g, "shield-ask")
    } else {
        let message = err.to_string();
        let code = if message.contains("HTTP 401") || message.contains("HTTP 403") {
            Code::Auth
        } else if message.contains("HTTP 402") {
            Code::Billing
        } else if message.contains("HTTP 404") {
            Code::NotFound
        } else if message.contains("HTTP 429") {
            Code::RateLimited
        } else {
            Code::Api
        };
        exit::err(code, message)
    }
}

pub fn dispatch(path: &[&str], m: &ArgMatches, g: &Globals) -> Option<Result<()>> {
    let command = path.join(" ");
    if matches!(
        command.as_str(),
        "shield ask" | "shield scan" | "shield redact" | "shield restructure" | "shield ingest" | "vault rehydrate"
    ) && words(m, &command).is_empty()
    {
        let mut root = crate::cli::build();
        let mut target = &mut root;
        for name in path {
            target = target.find_subcommand_mut(name).expect("path came from clap");
        }
        return Some(target.print_help().map_err(Into::into));
    }
    Some(match path {
        ["shield", "ask"] => ask(m, g),
        ["shield", "scan"] => scan(m, g),
        ["shield", "redact"] => redact(m, g),
        ["shield", "restructure"] => restructure(m),
        ["shield", "ingest"] => ingest(m, g),
        ["shield", "eval"] => eval(m, g),
        ["shield", "audit"] => audit(m),
        ["shield", "dict", "add"] => dict_add(m, g),
        ["shield", "dict", "list"] => dict_list(m, g),
        ["shield", "dict", "remove"] => dict_remove(m, g),
        ["vault", "list"] => vault_list(m),
        ["vault", "rehydrate"] => vault_rehydrate(m),
        ["vault", "rotate"] => {
            raw_json_out(&json!({"rotated":false,"reason":"rotation is deferred to preserve existing audit receipts"}))
        }
        ["vault", "purge"] => vault_purge(m, g),
        _ => return None,
    })
}

fn scan(m: &ArgMatches, g: &Globals) -> Result<()> {
    let text = required_file(&required(m, "shield scan")?)?;
    let runtime = Runtime::new(m, g, arg(m, "detector").unwrap_or("regex"))?;
    let db = store(m)?;
    let (entities, _) = runtime
        .detect(&text, &db.dictionary_terms()?)
        .map_err(|e| classify(e, g))?;
    let risk = shield::risk_score(&entities);
    raw_json_out(
        &json!({"risk":risk,"entities":if entities.is_empty() { Value::Null } else { Value::Array(entities.iter().map(entity_json).collect()) }}),
    )?;
    let max: i32 = arg(m, "max-risk")
        .unwrap_or("0")
        .parse()
        .map_err(|_| exit::usage("invalid --max-risk"))?;
    if risk > max {
        return Err(exit::err(
            Code::Partial,
            format!("PII risk {risk} exceeds --max-risk {max}"),
        ));
    }
    Ok(())
}

fn redact(m: &ArgMatches, g: &Globals) -> Result<()> {
    let mut db = store(m)?;
    let text = required_file(&required(m, "shield redact")?)?;
    let mut runtime = Runtime::new(m, g, arg(m, "detector").unwrap_or("regex"))?;
    runtime.extra_terms = columns(m, &text)?;
    let (redacted, _) = runtime
        .redact(&mut db, &text, bool_arg(m, "diff"))
        .map_err(|e| classify(e, g))?;
    if let Some(path) = arg(m, "output").filter(|s| !s.is_empty()) {
        return write_private(path, redacted.text.as_bytes());
    }
    if bool_arg(m, "diff") || g.json || g.agent {
        raw_json_out(&redaction_json(&redacted))
    } else {
        text_out(&redacted.text)
    }
}

fn restructure(m: &ArgMatches) -> Result<()> {
    let text = required_file(&required(m, "shield restructure")?)?;
    let out = shield::restructure(
        &text,
        &shield::RestructureOptions {
            bucket_numerics: bool_arg(m, "bucket"),
            coarsen_dates: arg(m, "coarsen-dates").unwrap_or("").into(),
            drop_columns: strings(m, "drop"),
        },
    )?;
    if let Some(path) = arg(m, "output").filter(|s| !s.is_empty()) {
        write_private(path, out.as_bytes())
    } else {
        text_out(&out)
    }
}

fn ingest(m: &ArgMatches, g: &Globals) -> Result<()> {
    let mut db = store(m)?;
    let input = required_file(&required(m, "shield ingest")?)?;
    let max = arg(m, "max-bytes")
        .unwrap_or("64000")
        .parse::<i64>()
        .map_err(|_| exit::usage("invalid --max-bytes"))?;
    let tranches = shield::split_records(&input, if max <= 0 { 64 * 1024 } else { max as usize })?;
    let runtime = Runtime::new(m, g, arg(m, "detector").unwrap_or("regex"))?;
    let mut corpus = String::new();
    let mut summaries = Vec::new();
    for t in &tranches {
        let (redacted, _) = runtime.redact(&mut db, &t.text, false).map_err(|e| classify(e, g))?;
        corpus.push_str(&redacted.text);
        let mut kinds = BTreeMap::<String, usize>::new();
        for e in &redacted.entities {
            *kinds.entry(e.kind.clone()).or_default() += 1;
        }
        summaries.push(json!({"index":t.index,"from_line":t.from_line,"to_line":t.to_line,"entity_count":redacted.entities.len(),"entity_kinds":kinds}));
    }
    let manifest = json!({"tranches":summaries});
    if let Some(path) = arg(m, "manifest").filter(|s| !s.is_empty()) {
        write_private(path, serde_json::to_string_pretty(&manifest)?.as_bytes())?;
    }
    let out = arg(m, "output").unwrap_or("");
    let mut summary = json!({"tranches":tranches.len(),"manifest":manifest,"output":out});
    if !out.is_empty() {
        write_private(out, corpus.as_bytes())?;
        return raw_json_out(&summary);
    }
    if g.json || g.agent {
        summary["masked_corpus"] = corpus.into();
        raw_json_out(&summary)
    } else {
        text_out(&corpus)
    }
}

#[derive(Debug)]
struct Prepared {
    payload: String,
    masked_entities: usize,
    counts: BTreeMap<String, usize>,
    leaks: Vec<Entity>,
    fallback: bool,
    tokens: Vec<String>,
}
fn prepare(db: &mut Store, runtime: &Runtime, data: &str, question: &str) -> Result<Prepared> {
    let (masked_data, data_fallback) = runtime.redact(db, data, false)?;
    let (masked_question, question_fallback) = runtime.redact(db, question, false)?;
    let payload = if data.is_empty() {
        format!("Question: {}", masked_question.text)
    } else {
        format!(
            "Use this masked corpus:\n\n{}\n\nQuestion: {}",
            masked_data.text, masked_question.text
        )
    };
    let mut counts = BTreeMap::new();
    let mut tokens = Vec::new();
    for e in masked_data.entities.iter().chain(&masked_question.entities) {
        *counts.entry(e.kind.clone()).or_insert(0) += 1;
        tokens.push(e.token.clone());
    }
    let mut tripwire_text = payload.clone();
    for token in &tokens {
        tripwire_text = tripwire_text.replace(token, &" ".repeat(token.len()));
    }
    let (leaks, tripwire_fallback) = runtime.detect(&tripwire_text, &db.dictionary_terms()?)?;
    Ok(Prepared {
        payload,
        masked_entities: tokens.len(),
        counts,
        leaks,
        fallback: data_fallback || question_fallback || tripwire_fallback,
        tokens,
    })
}

fn ask(m: &ArgMatches, g: &Globals) -> Result<()> {
    let question = required(m, "shield ask")?;
    let mut upstream = resolve_upstream(m)?;
    let explicit_model = arg(m, "model").filter(|s| !s.trim().is_empty());
    if upstream.name != "mixlayer" && explicit_model.is_none() {
        return Err(exit::usage(format!(
            "--model is required for --upstream {}",
            upstream.name
        )));
    }
    let detector_name = arg(m, "detector").unwrap_or("regex");
    if detector_name == "regex" && upstream.name != "mixlayer" && !bool_arg(m, "accept-regex-only") {
        return Err(exit::usage(
            "regex detection misses names, addresses and international IDs; use --detector local:<model> or mixlayer:<model> (or pass --accept-regex-only)",
        ));
    }
    let warnings = if detector_name == "regex" && upstream.name == "mixlayer" {
        vec![
            "regex detection misses names, addresses and international IDs; use --detector local:<model> or mixlayer:<model>",
        ]
    } else {
        Vec::new()
    };
    if !g.json && !g.agent {
        for warning in &warnings {
            eprintln!("warning: {warning}");
        }
    }
    if upstream.name == "mixlayer" {
        let client = commands::new_client(m)?;
        upstream.base_url = client.config.base_url;
        let url = url::Url::parse(&upstream.base_url)?;
        upstream.host = url[url::Position::BeforeHost..url::Position::AfterPort].into();
    }
    if g.dry_run && detector_name != "regex" {
        return Err(exit::usage(
            "--dry-run requires --detector regex because model detection makes a network call",
        ));
    }
    let mut db = store(m)?;
    let catalog = if upstream.name == "mixlayer" && !g.dry_run {
        models::load_live_catalog(m).ok()
    } else {
        None
    };
    let model = explicit_model.map(str::to_owned).unwrap_or_else(|| {
        catalog
            .as_ref()
            .map(|(rows, source)| models::default_model(rows, source))
            .unwrap_or_else(|| "z-ai/glm-5.3".into())
    });
    let data = match arg(m, "data").filter(|s| !s.is_empty()) {
        Some(path) => file(path)?,
        None => String::new(),
    };
    let mut runtime = Runtime::new(m, g, detector_name)?;
    runtime.extra_terms = columns(m, &data)?;
    let prepared = prepare(&mut db, &runtime, &data, &question).map_err(|e| classify(e, g))?;
    if g.dry_run {
        let mut preview = json!({"dry_run":true,"destination_host":upstream.host,"model":model,"payload":prepared.payload,"messages":[{"role":"system","content":PLACEHOLDER_INSTRUCTION},{"role":"user","content":prepared.payload}],"masked_counts":prepared.counts,"tripwire_hits":prepared.leaks.iter().map(entity_json).collect::<Vec<_>>(),"tripwire_detector":detector_name,"warnings":warnings});
        if upstream.name == "mixlayer" {
            preview["catalog_source"] = if explicit_model.is_some() {
                "explicit"
            } else {
                "offline_fallback"
            }
            .into();
        }
        if g.json || g.agent {
            return json_out(&preview, g);
        }
        return text_out(&format!(
            "Destination host: {}\nModel: {}\nSystem: {}\nPayload:\n{}\nMasked counts: {:?}\nTripwire hits: {}\n",
            upstream.host,
            model,
            PLACEHOLDER_INSTRUCTION,
            prepared.payload,
            prepared.counts,
            prepared.leaks.len()
        ));
    }
    if !prepared.leaks.is_empty() && !bool_arg(m, "force") {
        return Err(exit::err(
            Code::Partial,
            format!(
                "masked payload still contains {} detector hits; use --force to send anyway",
                prepared.leaks.len()
            ),
        ));
    }
    let detector_catalog = if detector_name.starts_with("mixlayer:") && catalog.is_none() {
        models::load_live_catalog(m).ok()
    } else {
        None
    };
    let detector_cost = if detector_name.starts_with("mixlayer:") {
        let usage = runtime.usage.borrow();
        let use_json = json!({"prompt_tokens":usage.prompt,"completion_tokens":usage.completion,"prompt_tokens_details":{"cached_tokens":usage.cache}});
        catalog
            .as_ref()
            .or(detector_catalog.as_ref())
            .and_then(|(rows, _)| models::price(detector_name.trim_start_matches("mixlayer:"), rows, &use_json))
            .filter(|cost| !usage.unknown || *cost == 0.0)
    } else {
        None
    };
    let detector_cost_unknown = detector_name.starts_with("mixlayer:") && detector_cost.is_none();
    let (run, raw) = save_chat(
        m,
        g,
        &mut db,
        &upstream,
        &model,
        &prepared.payload,
        bool_arg(m, "show-thinking"),
        detector_cost,
        detector_cost_unknown,
        catalog.as_ref(),
    )?;
    let allowed = prepared.tokens.iter().map(String::as_str).collect::<Vec<_>>();
    let restored = shield::rehydrate_checked_for_tokens(&db, &run.answer, &allowed)?;
    let reasoning = if run.reasoning.is_empty() {
        String::new()
    } else {
        shield::rehydrate_checked_for_tokens(&db, &run.reasoning, &allowed)?.text
    };
    let audit = AuditRecord {
        id: id("audit")?,
        run_id: run.id.clone(),
        command: "shield ask".into(),
        payload_sha256: hex::encode(Sha256::digest(prepared.payload.as_bytes())),
        byte_count: prepared.payload.len() as i64,
        model: model.clone(),
        guard_model: arg(m, "guard").unwrap_or("qwen/qwen3.5-4b-free").into(),
        masked_entities: prepared.masked_entities as i64,
        leaked_pii_count: prepared.leaks.len() as i64,
        cost_usd: (!run.cost_unknown).then_some(run.cost_usd),
        upstream_host: upstream.host.clone(),
        detector: detector_name.into(),
        detector_host: runtime.host.clone(),
        detector_fallback: if prepared.fallback {
            "regex".into()
        } else {
            String::new()
        },
        detector_notice: runtime.notice.clone(),
        tripwire_detector: detector_name.into(),
        ignored_model_spans: runtime.ignored_spans.get() as i64,
        detector_cost_usd: detector_cost,
        masked_counts: Some(prepared.counts.iter().map(|(k, v)| (k.clone(), *v as i64)).collect()),
        restored_count: restored.restored_count as i64,
        unrestored_count: restored.unrestored_count as i64,
        unrestored_tokens: Some(restored.unrestored_tokens.clone()),
        repairs: Some(restored.repairs.clone()),
        created_at: Some(chrono::Utc::now()),
    };
    db.save_audit(audit.clone())?;
    if g.json || g.agent {
        let mut run_json = raw;
        run_json["answer"] = restored.text.clone().into();
        if !reasoning.is_empty() {
            run_json["reasoning"] = reasoning.into();
        }
        return json_out(
            &json!({"answer":restored.text,"run":run_json,"audit":audit_json(&audit),"warnings":warnings,"restored_count":restored.restored_count,"unrestored_count":restored.unrestored_count,"unrestored_tokens":restored.unrestored_tokens,"repairs":restored.repairs}),
            g,
        );
    }
    println!("{}", restored.text);
    if restored.unrestored_count > 0 {
        eprintln!(
            "warning: {} placeholder-like tokens could not be restored: {}",
            restored.unrestored_count,
            restored.unrestored_tokens.join(", ")
        );
    }
    if bool_arg(m, "show-sent") {
        eprintln!(
            "{}\n{}",
            color(&format!("sent to {}:", upstream.host), 2, g),
            color(&prepared.payload, 2, g)
        );
    }
    let n: usize = prepared.counts.values().sum();
    if n > 0 {
        let kinds = format_masked_counts(&prepared.counts);
        let message = format!(
            "masked {n} before sending ({kinds}), restored {} locally",
            restored.restored_count
        );
        eprintln!(
            "{} {}",
            color(&message, 32, g),
            color(&format!("· receipt {}", audit.id), 2, g)
        );
    } else {
        eprintln!(
            "{} {}",
            color("nothing to mask", 33, g),
            color(&format!("· receipt {}", audit.id), 2, g)
        );
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn save_chat(
    m: &ArgMatches,
    g: &Globals,
    db: &mut Store,
    upstream: &Upstream,
    model: &str,
    payload: &str,
    thinking: bool,
    detector_cost: Option<f64>,
    detector_cost_unknown: bool,
    catalog: Option<&(Vec<Value>, String)>,
) -> Result<(RunRecord, Value)> {
    let messages = json!([{"role":"system","content":PLACEHOLDER_INSTRUCTION},{"role":"user","content":payload}]);
    let result = if upstream.name == "mixlayer" && upstream.key_env == "MIXLAYER_API_KEY" {
        let mut client = commands::new_client(m)?;
        let start = Instant::now();
        let response = client
            .post(
                "/chat/completions",
                &json!({"model":model,"messages":messages,"thinking":thinking}),
            )
            .map_err(|e| classify(e, g))?;
        let answer = response["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string();
        if answer.is_empty() {
            return Err(exit::err(Code::Api, "upstream chat response has no answer"));
        }
        runtime::ChatResult {
            answer,
            reasoning: response["choices"][0]["message"]["reasoning_content"]
                .as_str()
                .unwrap_or("")
                .into(),
            usage: response["usage"].clone(),
            raw: response,
            latency_ms: start.elapsed().as_millis() as i64,
        }
    } else {
        let key = key_from_env(&upstream.key_env, false)?;
        let timeout = crate::api::client::parse_duration(arg(m, "timeout").unwrap_or("1m0s"))?;
        let body = json!({"model":model,"messages":messages});
        external_chat(
            &upstream.base_url,
            &key,
            &body,
            if timeout.is_zero() {
                std::time::Duration::from_secs(60)
            } else {
                timeout
            },
        )
        .map_err(|e| classify(e, g))?
    };
    let own_catalog = if upstream.name == "mixlayer" && catalog.is_none() {
        models::load_live_catalog(m).ok()
    } else {
        None
    };
    let catalog = catalog.or(own_catalog.as_ref());
    let cost = if upstream.name == "mixlayer" {
        catalog.and_then(|(rows, _)| models::price(model, rows, &result.usage))
    } else {
        None
    };
    let cost = cost
        .filter(|_| !detector_cost_unknown)
        .map(|c| c + detector_cost.unwrap_or(0.0));
    let source = if upstream.name == "mixlayer" {
        catalog.map(|(_, source)| source.as_str()).unwrap_or("offline_fallback")
    } else {
        ""
    };
    let run = RunRecord {
        id: id("run")?,
        command: "shield ask".into(),
        prompt: payload.into(),
        answer: result.answer.clone(),
        reasoning: result.reasoning.clone(),
        model: model.into(),
        params_json: Some(json!({"model":model,"upstream":upstream.name,"reasoning":thinking})),
        raw_json: Some(result.raw.clone()),
        prompt_tokens: result.usage["prompt_tokens"].as_i64().unwrap_or(0),
        completion_tokens: result.usage["completion_tokens"].as_i64().unwrap_or(0),
        total_tokens: result.usage["total_tokens"].as_i64().unwrap_or(0),
        cost_usd: cost.unwrap_or(0.0),
        cost_unknown: cost.is_none(),
        catalog_source: source.into(),
        latency_ms: result.latency_ms,
        ..Default::default()
    };
    db.save_run(run.clone())?;
    let mut output_run = json!({"id":run.id,"command":"shield ask","prompt":payload,"answer":result.answer,"model":model,"params_json":run.params_json,"raw_json":result.raw,"cost_usd":cost,"created_at":"0001-01-01T00:00:00Z"});
    if !source.is_empty() {
        output_run["catalog_source"] = source.into();
    }
    if run.latency_ms != 0 {
        output_run["latency_ms"] = run.latency_ms.into();
    }
    if !run.reasoning.is_empty() {
        output_run["reasoning"] = run.reasoning.clone().into();
    }
    if run.cost_unknown {
        output_run["cost_unknown"] = true.into();
    }
    for (key, value) in [
        ("prompt_tokens", run.prompt_tokens),
        ("completion_tokens", run.completion_tokens),
        ("total_tokens", run.total_tokens),
    ] {
        if value != 0 {
            output_run[key] = value.into();
        }
    }
    Ok((run, output_run))
}

fn audit_json(a: &AuditRecord) -> Value {
    let mut out = json!({"id":a.id,"run_id":a.run_id,"command":a.command,"payload_sha256":a.payload_sha256,"byte_count":a.byte_count,"model":a.model,"guard_model":a.guard_model,"masked_entities":a.masked_entities,"leaked_pii_count":a.leaked_pii_count,"cost_usd":a.cost_usd,"created_at":a.created_at.map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true)),"upstream_host":a.upstream_host,"detector":a.detector,"detector_host":a.detector_host,"detector_fallback":a.detector_fallback,"detector_notice":a.detector_notice,"tripwire_detector":a.tripwire_detector,"ignored_model_spans":a.ignored_model_spans,"detector_cost_usd":a.detector_cost_usd,"masked_counts":a.masked_counts,"restored_count":a.restored_count,"unrestored_count":a.unrestored_count,"unrestored_tokens":a.unrestored_tokens,"repairs":a.repairs});
    for key in [
        "run_id",
        "model",
        "guard_model",
        "upstream_host",
        "detector",
        "detector_host",
        "detector_fallback",
        "detector_notice",
        "tripwire_detector",
    ] {
        if out[key] == "" {
            out.as_object_mut().unwrap().remove(key);
        }
    }
    if a.masked_counts.as_ref().is_none_or(HashMap::is_empty) {
        out.as_object_mut().unwrap().remove("masked_counts");
    }
    if a.repairs.as_ref().is_none_or(Vec::is_empty) {
        out.as_object_mut().unwrap().remove("repairs");
    }
    out
}

fn audit(m: &ArgMatches) -> Result<()> {
    let db = store(m)?;
    let id = words(m, "shield audit").first().cloned().unwrap_or_default();
    let limit = arg(m, "limit")
        .unwrap_or("50")
        .parse::<i64>()
        .map_err(|_| exit::usage("invalid --limit"))?;
    let rows = db.audit_records(&id, limit)?;
    raw_json_out(&Value::Array(rows.iter().map(audit_json).collect()))
}
fn dict_add(m: &ArgMatches, g: &Globals) -> Result<()> {
    let term = required(m, "shield dict add")?;
    let kind = arg(m, "kind").unwrap_or("").trim().to_uppercase();
    if !shield::valid_kind(&kind) || term.trim().is_empty() {
        return Err(exit::usage("a nonempty term and valid --kind are required"));
    }
    store(m)?.add_dictionary_term(&term, &kind)?;
    json_out(&json!({"term":term.trim(),"kind":kind}), g)
}
fn dict_list(m: &ArgMatches, g: &Globals) -> Result<()> {
    let terms = store(m)?.dictionary_terms()?;
    if terms.is_empty() {
        return json_out(&Value::Null, g);
    }
    json_out(
        &json!(
            terms
                .iter()
                .map(|t| json!({"term":t.term,"kind":t.kind}))
                .collect::<Vec<_>>()
        ),
        g,
    )
}
fn dict_remove(m: &ArgMatches, g: &Globals) -> Result<()> {
    let term = required(m, "shield dict remove")?;
    json_out(&json!({"removed":store(m)?.remove_dictionary_term(&term)?}), g)
}
fn vault_list(m: &ArgMatches) -> Result<()> {
    let rows = store(m)?.vault_entries(false)?;
    raw_json_out(
        &json!(
            rows.iter()
                .map(|e| json!({"token":e.token,"kind":e.kind,"created_at":e.created_at.map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))}))
                .collect::<Vec<_>>()
        ),
    )
}
fn vault_rehydrate(m: &ArgMatches) -> Result<()> {
    let input = text_or_file(&required(m, "vault rehydrate")?)?;
    text_out(&shield::rehydrate(&store(m)?, &input)?)
}
fn vault_purge(m: &ArgMatches, g: &Globals) -> Result<()> {
    if !g.yes && !g.agent {
        return Err(exit::usage("pass --yes to purge the vault"));
    }
    store(m)?.purge_vault()?;
    raw_json_out(&json!({"purged":true}))
}

fn eval_metrics(c: EvalCounts) -> EvalMetrics {
    EvalMetrics {
        recall: if c.gold > 0 {
            c.found as f64 / c.gold as f64
        } else {
            1.0
        },
        precision: if c.detected > 0 {
            c.correct as f64 / c.detected as f64
        } else {
            1.0
        },
        leak_rate: if c.gold > 0 {
            c.leaked as f64 / c.gold as f64
        } else {
            0.0
        },
        counts: c,
    }
}
fn eval_one(samples: &[EvalSample], runtime: &Runtime) -> Result<EvalResult> {
    let mut counts = BTreeMap::<String, EvalCounts>::new();
    let mut errors = 0;
    let mut vault = shield::MemoryVault::default();
    for sample in samples {
        let detection = runtime.detect(&sample.text, &[]);
        let entities = match detection {
            Ok((entities, false)) => entities,
            _ => {
                errors += 1;
                Vec::new()
            }
        };
        let redacted = if entities.is_empty() {
            sample.text.clone()
        } else {
            shield::redact_entities(&mut vault, &sample.text, entities.clone(), false)?.text
        };
        let mut gold_spans = Vec::new();
        for gold in &sample.entities {
            if !gold.text.is_empty() {
                for (start, _) in sample.text.match_indices(&gold.text) {
                    gold_spans.push((gold.kind.as_str(), start, start + gold.text.len()));
                }
            }
        }
        for &(kind, start, end) in &gold_spans {
            let c = counts.entry(kind.into()).or_default();
            c.gold += 1;
            if entities.iter().any(|e| e.start <= start && e.end >= end) {
                c.found += 1;
            }
            if redacted.contains(&sample.text[start..end]) {
                c.leaked += 1;
            }
        }
        for e in &entities {
            let c = counts.entry(e.kind.clone()).or_default();
            c.detected += 1;
            if gold_spans.iter().any(|&(_, start, end)| e.start < end && start < e.end) {
                c.correct += 1;
            }
        }
    }
    let mut overall = EvalCounts::default();
    let by_kind = counts
        .into_iter()
        .map(|(kind, c)| {
            overall.gold += c.gold;
            overall.found += c.found;
            overall.detected += c.detected;
            overall.correct += c.correct;
            overall.leaked += c.leaked;
            (kind, eval_metrics(c))
        })
        .collect();
    Ok(EvalResult {
        detector: runtime.name.clone(),
        errors,
        overall: eval_metrics(overall),
        by_kind,
    })
}
fn eval(m: &ArgMatches, g: &Globals) -> Result<()> {
    let sample_path = arg(m, "sample")
        .filter(|s| !s.is_empty())
        .ok_or_else(|| exit::usage("--sample is required"))?;
    let min = arg(m, "min-recall")
        .unwrap_or("0.9")
        .parse::<f64>()
        .map_err(|_| exit::usage("invalid --min-recall"))?;
    if !(0.0..=1.0).contains(&min) {
        return Err(exit::usage("--min-recall must be between 0 and 1"));
    }
    let samples = shield::read_eval_samples(&required_file(sample_path)?)?;
    let mut names = strings(m, "detector");
    if names.is_empty() {
        names.push("regex".into());
    }
    let mut results = Vec::new();
    let mut below = false;
    for name in names {
        let runtime = Runtime::new(m, g, &name)?;
        let result = eval_one(&samples, &runtime)?;
        below |= result.overall.recall < min;
        results.push(result);
    }
    if g.json || g.agent {
        json_out(&json!({"results":results,"min_recall":min}), g)?;
    } else {
        println!("DETECTOR\tRECALL\tPRECISION\tLEAK_RATE\tGOLD\tERRORS");
        for r in &results {
            println!(
                "{}\t{:.3}\t{:.3}\t{:.3}\t{}\t{}",
                r.detector, r.overall.recall, r.overall.precision, r.overall.leak_rate, r.overall.counts.gold, r.errors
            );
        }
    }
    if below {
        return Err(exit::usage(format!("detector recall below --min-recall {min:.3}")));
    }
    Ok(())
}

/// "6 CUSTOMER, 6 EMAIL, 5 COMPANY": largest count first, then by name,
/// skipping zeros (same as the Go CLI's formatMaskedCounts).
fn format_masked_counts<'a>(counts: impl IntoIterator<Item = (&'a String, &'a usize)>) -> String {
    let mut items: Vec<(&String, usize)> = counts
        .into_iter()
        .filter(|(_, v)| **v > 0)
        .map(|(k, v)| (k, *v))
        .collect();
    items.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    items
        .iter()
        .map(|(k, v)| format!("{v} {k}"))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod masked_counts_tests {
    #[test]
    fn largest_first_then_name() {
        let m: std::collections::BTreeMap<String, usize> = [("EMAIL", 1), ("PERSON", 2), ("PHONE", 0), ("CARD", 1)]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect();
        assert_eq!(super::format_masked_counts(&m), "2 PERSON, 1 CARD, 1 EMAIL");
    }
}
