use std::time::Duration;

use clap::ArgMatches;
use serde_json::{Value, json};

use super::{bool_arg, classified, new_client, print, words};
use crate::Globals;
use crate::api::client::{ApiError, Client, parse_duration};
use crate::exit;
use crate::store::{ModelCacheRecord, Store};
use sha2::{Digest, Sha256};

fn catalog_items(value: &Value) -> Vec<Value> {
    if let Some(array) = value.as_array() {
        return array
            .iter()
            .filter_map(|v| match v {
                Value::Object(_) => Some(v.clone()),
                Value::String(id) => Some(json!({"id": id})),
                _ => None,
            })
            .collect();
    }
    for key in ["data", "models", "results", "organizations"] {
        if let Some(nested) = value.get(key) {
            return catalog_items(nested);
        }
    }
    if value.is_object() {
        vec![value.clone()]
    } else {
        Vec::new()
    }
}

pub fn list(m: &ArgMatches, g: &Globals) -> anyhow::Result<()> {
    if bool_arg(m, "live-prices") {
        return live_prices(m, g);
    }
    if g.dry_run {
        return print(&json!({"results": {"dry_run": true}, "meta": {"source": "live"}}), g);
    }
    let mut client = new_client(m)?;
    let data = client.get_raw("/models").map_err(|e| classified(e, g, "models-list"))?;
    crate::output::print_endpoint(&data, g, 0)
}

pub fn get(m: &ArgMatches, g: &Globals) -> anyhow::Result<()> {
    let args = words(m, "models get");
    let Some(id) = args.first().filter(|s| !s.is_empty()) else {
        return Err(exit::usage("id is required"));
    };
    if g.dry_run {
        return print(
            &json!({"dry_run": true, "method": "GET", "endpoint": "/models", "select_id": id}),
            g,
        );
    }
    let mut client = new_client(m)?;
    let data = client.get_raw("/models").map_err(|e| classified(e, g, "models-get"))?;
    let item = raw_model(&data, id);
    let item = match item {
        Some(v) => v,
        None => {
            let e = ApiError {
                method: "GET".into(),
                path: format!("/models/{id}"),
                status: 404,
                body: format!("{{\"error\":{{\"message\":\"Model '{id}' not found\"}}}}"),
                retry_after: String::new(),
            };
            return Err(classified(e.into(), g, "models-get"));
        }
    };
    crate::output::print_endpoint(&item, g, 0)
}

// GetModel in Go returns the selected RawMessage, including wire whitespace.
fn raw_model(raw: &str, id: &str) -> Option<String> {
    if let Ok(items) = serde_json::from_str::<Vec<&serde_json::value::RawValue>>(raw) {
        return items.into_iter().find_map(|item| {
            let value: Value = serde_json::from_str(item.get()).ok()?;
            (value.get("id").and_then(Value::as_str) == Some(id)).then(|| item.get().to_owned())
        });
    }
    let object: serde_json::Map<String, Value> = serde_json::from_str(raw).ok()?;
    let fields: std::collections::BTreeMap<String, &serde_json::value::RawValue> = serde_json::from_str(raw).ok()?;
    for key in ["data", "models", "results", "organizations"] {
        if let Some(nested) = fields.get(key) {
            return raw_model(nested.get(), id);
        }
    }
    (object.get("id").and_then(Value::as_str) == Some(id)).then(|| raw.to_owned())
}

fn live_prices(m: &ArgMatches, g: &Globals) -> anyhow::Result<()> {
    if g.dry_run {
        return print(
            &json!({"endpoint": "/v1/organizations/{org}/models", "fallback_endpoint": "/v1/models", "dry_run": true}),
            g,
        );
    }
    let mut store = open_store(m)?;
    let (models, source, warning) = match load_live_catalog_from_store(m, &mut store) {
        Ok((models, source)) => (models, source, None),
        Err(e) => (offline_live_models(), "offline_fallback".into(), Some(e.to_string())),
    };
    let mut result = json!({"models": models, "source": source});
    if let Some(warning) = warning.as_ref() {
        result["warning"] = warning.as_str().into();
    }
    if g.json || g.agent {
        let rendered = crate::output::format_output(&result.to_string(), g, false, false)?;
        crate::output::write_stdout(rendered.stdout.as_bytes())
    } else {
        if let Some(warning) = warning {
            eprintln!("warning: {warning}");
        }
        let mut rows = vec![vec!["MODEL".into(), "STATUS".into(), "OUTPUT USD/1M".into()]];
        rows.extend(models.iter().map(|model| {
            vec![
                model["model_sku"].as_str().unwrap_or("").into(),
                model["status"].as_str().unwrap_or("").into(),
                model["price_m_output_tokens_usd"]
                    .as_f64()
                    .map(|p| format!("{p:.4}"))
                    .unwrap_or_else(|| "unknown".into()),
            ]
        }));
        crate::output::write_stdout(crate::output::table(&rows).as_bytes())
    }
}

fn offline_live_models() -> Vec<Value> {
    [
        "qwen/qwen3.5-4b-free",
        "qwen/qwen3.5-9b",
        "qwen/qwen3.6-35b-a3b",
        "qwen/qwen3.8-27b",
        "z-ai/glm-5.3",
    ]
    .iter()
    .map(|id| {
        json!({
            "model_sku": id, "price_m_input_tokens_usd": null, "price_m_output_tokens_usd": null,
            "price_m_input_cache_read_tokens_usd": null, "max_context_len": 0,
            "tools_supported": false, "input_modalities": null, "status": "offline fallback", "access": null
        })
    })
    .collect()
}

pub(crate) fn load_live_catalog(m: &ArgMatches) -> anyhow::Result<(Vec<Value>, String)> {
    load_live_catalog_from_store(m, &mut open_store(m)?)
}

fn open_store(m: &ArgMatches) -> anyhow::Result<Store> {
    match m.try_get_one::<String>("db").ok().flatten().filter(|p| !p.is_empty()) {
        Some(path) => Store::open(path),
        None => Store::open_default(),
    }
}

fn load_live_catalog_from_store(m: &ArgMatches, store: &mut Store) -> anyhow::Result<(Vec<Value>, String)> {
    let config = new_client(m)?.config;
    if config.auth_header.is_empty() {
        anyhow::bail!("MIXLAYER_API_KEY is not configured");
    }
    let requested = parse_duration(super::arg(m, "timeout").unwrap_or("1m0s"))?;
    let timeout = if requested.is_zero() {
        Duration::from_secs(5)
    } else {
        requested.min(Duration::from_secs(5))
    };
    let mut client = Client::new(config, timeout, 0.0, true)?;
    let base = crate::api::platform_base()?;
    // Platform prices use a separate origin and a five-second probe timeout.
    client.config.base_url = base.clone();
    let scope = catalog_scope(&base, &client.config.auth_header);
    cached_or_fetch(store, &scope, || fetch_live_catalog(&mut client))
}

fn catalog_scope(base: &str, auth: &str) -> String {
    format!(
        "{}:{}",
        base.trim_end_matches('/'),
        hex::encode(&Sha256::digest(auth.as_bytes())[..8])
    )
}

fn cached_or_fetch(
    store: &mut Store,
    scope: &str,
    fetch: impl FnOnce() -> anyhow::Result<(Vec<Value>, String)>,
) -> anyhow::Result<(Vec<Value>, String)> {
    if let Some(raw) = store.cached_live_catalog(scope, Duration::from_secs(3600))?
        && let Ok(catalog) = serde_json::from_value::<LiveCatalog>(raw)
        && !catalog.models.is_empty()
    {
        return Ok((
            catalog.models.into_iter().map(live_model_value).collect(),
            catalog.source,
        ));
    }
    let (models, source) = fetch()?;
    if models.is_empty() {
        anyhow::bail!("live catalog returned no models");
    }
    let _ = store.save_live_catalog(scope, &json!({"models": models, "source": source}));
    Ok((models, source))
}

fn fetch_live_catalog(client: &mut Client) -> anyhow::Result<(Vec<Value>, String)> {
    let organizations = client.probe("/v1/organizations");
    if let Ok(orgs) = &organizations {
        for org in catalog_items(orgs) {
            let id = string_field(&org, &["id", "slug", "organization_id"]);
            if !id.is_empty()
                && let Ok(prices) = client.probe(&format!(
                    "/v1/organizations/{}/models",
                    url::form_urlencoded::byte_serialize(id.as_bytes()).collect::<String>()
                ))
            {
                let models = parse_live_catalog(&prices, "organization");
                if !models.is_empty() {
                    return Ok((models, "organization".into()));
                }
            }
        }
    }
    let raw = client.probe("/v1/models").map_err(|error| match organizations {
        Err(org_error) => anyhow::anyhow!("live catalog: {org_error}; fallback: {error}"),
        _ => error,
    })?;
    Ok((parse_live_catalog(&raw, "ids_only"), "ids_only".into()))
}

#[derive(Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct LiveModel {
    model_sku: String,
    price_m_input_tokens_usd: Option<f64>,
    price_m_output_tokens_usd: Option<f64>,
    price_m_input_cache_read_tokens_usd: Option<f64>,
    max_context_len: i64,
    tools_supported: bool,
    input_modalities: Option<Vec<String>>,
    status: String,
    access: Value,
    #[serde(skip_serializing_if = "String::is_empty")]
    model_type: String,
}

#[derive(Default, serde::Deserialize)]
#[serde(default)]
struct LiveCatalog {
    models: Vec<LiveModel>,
    source: String,
}

fn live_model_value(model: LiveModel) -> Value {
    let mut value = serde_json::to_value(model).expect("live model");
    for key in [
        "price_m_input_tokens_usd",
        "price_m_output_tokens_usd",
        "price_m_input_cache_read_tokens_usd",
    ] {
        if let Some(n) = value[key].as_f64() {
            value[key] = json_number(n);
        }
    }
    value
}

fn parse_live_catalog(raw: &Value, source: &str) -> Vec<Value> {
    catalog_items(raw)
        .iter()
        .filter_map(|item| {
            let sku = string_field(item, &["model_sku", "id", "model", "name"]);
            if sku.is_empty() {
                return None;
            }
            let modalities: Vec<_> = item["input_modalities"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect();
            let status = string_field(item, &["status"]);
            let price = |key| {
                if source == "organization" {
                    number(item.get(key))
                } else {
                    None
                }
            };
            let model = LiveModel {
                model_sku: sku.into(),
                price_m_input_tokens_usd: price("price_m_input_tokens_usd"),
                price_m_output_tokens_usd: price("price_m_output_tokens_usd"),
                price_m_input_cache_read_tokens_usd: price("price_m_input_cache_read_tokens_usd"),
                max_context_len: numeric_field(item, &["max_context_len", "context_window"]).unwrap_or(0.0) as i64,
                tools_supported: bool_field(item, false, &["tools_supported"]),
                input_modalities: (!modalities.is_empty()).then_some(modalities),
                status: if status.is_empty() && source == "ids_only" {
                    "unknown"
                } else {
                    status
                }
                .into(),
                access: item["access"].clone(),
                model_type: string_field(item, &["model_type", "type"]).into(),
            };
            Some(live_model_value(model))
        })
        .collect()
}

fn number(value: Option<&Value>) -> Option<f64> {
    value.and_then(|v| v.as_f64().or_else(|| v.as_str().and_then(|s| s.parse().ok())))
}

pub(crate) fn default_model(models: &[Value], source: &str) -> String {
    let mut best = None;
    let mut max = -1.0;
    for model in models {
        let sku = model["model_sku"].as_str().unwrap_or("");
        if !model["status"].as_str().unwrap_or("").eq_ignore_ascii_case("active")
            || sku.to_ascii_lowercase().contains("embed")
            || model["model_type"]
                .as_str()
                .unwrap_or("")
                .to_ascii_lowercase()
                .contains("embed")
        {
            continue;
        }
        if let Some(price) = model["price_m_output_tokens_usd"].as_f64()
            && price > max
        {
            max = price;
            best = Some(sku.to_owned());
        }
    }
    if let Some(best) = best {
        return best;
    }
    if source == "ids_only" {
        if models.iter().any(|m| m["model_sku"] == "z-ai/glm-5.3") {
            return "z-ai/glm-5.3".into();
        }
        if let Some(first) = models.first().and_then(|m| m["model_sku"].as_str()) {
            return first.into();
        }
    }
    "z-ai/glm-5.3".into()
}

pub(crate) fn price(model: &str, models: &[Value], usage: &Value) -> Option<f64> {
    let row = models.iter().find(|m| m["model_sku"] == model)?;
    let input = row["price_m_input_tokens_usd"].as_f64()?;
    let output = row["price_m_output_tokens_usd"].as_f64()?;
    let reported = usage.get("prompt_tokens").is_some() && usage.get("completion_tokens").is_some();
    if !reported && (input != 0.0 || output != 0.0) {
        return None;
    }
    let prompt = usage["prompt_tokens"].as_i64().unwrap_or(0).max(0);
    let completion = usage["completion_tokens"].as_i64().unwrap_or(0).max(0);
    let cache = ["cache_read_input_tokens", "input_cache_read_tokens"]
        .iter()
        .filter_map(|k| usage.get(*k).and_then(Value::as_i64))
        .chain(usage["prompt_tokens_details"]["cached_tokens"].as_i64())
        .max()
        .unwrap_or(0)
        .clamp(0, prompt);
    let cache_price = row["price_m_input_cache_read_tokens_usd"].as_f64();
    if cache > 0 && cache_price.is_none() && input != 0.0 {
        return None;
    }
    let cost =
        ((prompt - cache) as f64 * input + completion as f64 * output + cache as f64 * cache_price.unwrap_or(0.0))
            / 1_000_000.0;
    Some((cost * 1e12).round() / 1e12)
}

pub fn query(m: &ArgMatches, g: &Globals) -> anyhow::Result<()> {
    let args = words(m, "models query");
    let Some(dsl) = args.first() else {
        return Err(exit::usage("query DSL is required"));
    };
    if g.dry_run {
        return print(&json!({"dry_run": true, "endpoint": "/models", "query": dsl}), g);
    }
    let mut store = open_store(m)?;
    if bool_arg(m, "refresh") {
        let mut client = new_client(m)?;
        let catalog = client
            .get_no_cache("/models")
            .map_err(|e| classified(e, g, "models-query"))?;
        refresh_model_cache(&mut store, &catalog)?;
    } else {
        seed_model_cache(&mut store)?;
    }
    let records = store.query_models(dsl)?.iter().map(record_value).collect();
    print(&Value::Array(records), &Globals::default())
}

fn seed_model_cache(store: &mut Store) -> anyhow::Result<()> {
    if !store.all_models()?.is_empty() {
        return Ok(());
    }
    for record in offline_records() {
        store.save_model_cache(&record)?;
    }
    Ok(())
}

fn refresh_model_cache(store: &mut Store, catalog: &Value) -> anyhow::Result<()> {
    let records = records_from_catalog(catalog);
    if records.is_empty() {
        anyhow::bail!("models refresh returned no model IDs");
    }
    for record in records {
        store.save_model_cache(&record)?;
    }
    Ok(())
}

fn record_value(record: &ModelCacheRecord) -> Value {
    // Go emits struct fields in declaration order, and float zeros as 0.
    json!({
        "id": record.id, "context_window": record.context_window,
        "supports_tools": record.supports_tools, "supports_reasoning": record.supports_reasoning,
        "input_price_per_million": json_number(record.input_price_per_million),
        "output_price_per_million": json_number(record.output_price_per_million),
        "raw_json": record.raw_json,
        "updated_at": record.updated_at.map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
    })
}

fn json_number(n: f64) -> Value {
    if n.fract() == 0.0 && n >= i64::MIN as f64 && n < i64::MAX as f64 {
        Value::from(n as i64)
    } else {
        Value::from(n)
    }
}

fn known(id: &str) -> (i64, bool, bool, f64, f64) {
    match id {
        "qwen/qwen3.5-4b-free" => (131072, true, true, 0.0, 0.0),
        "qwen/qwen3.5-9b" => (131072, true, true, 0.10, 0.40),
        "qwen/qwen3.6-35b-a3b" => (131072, true, true, 0.25, 1.30),
        "qwen/qwen3.8-27b" => (131072, true, true, 0.25, 1.95),
        "z-ai/glm-5.3" => (262144, true, true, 1.15, 3.90),
        _ => (0, false, false, 0.0, 0.0),
    }
}

fn record(item: &Value) -> Option<ModelCacheRecord> {
    let id = string_field(item, &["id", "name", "model"]);
    if id.is_empty() {
        return None;
    }
    let (known_ctx, known_tools, known_reasoning, known_input, known_output) = known(id);
    let ctx = ["context_window", "context_length", "context_size", "max_context_tokens"]
        .iter()
        .find_map(|k| item.get(k).filter(|v| v.is_number()).map(|v| v.as_i64().unwrap_or(0)))
        .filter(|n| *n > 0)
        .unwrap_or(known_ctx);
    let mut raw = item.clone();
    raw.sort_all_objects();
    Some(ModelCacheRecord {
        id: id.into(),
        context_window: ctx,
        supports_tools: bool_field(
            item,
            known_tools,
            &["supports_tools", "tools", "tool_use", "function_calling"],
        ),
        supports_reasoning: bool_field(item, known_reasoning, &["supports_reasoning", "reasoning", "thinking"]),
        input_price_per_million: numeric_field(item, &["input_price_per_million", "prompt_price_per_million"])
            .filter(|n| *n > 0.0)
            .unwrap_or(known_input),
        output_price_per_million: numeric_field(item, &["output_price_per_million", "completion_price_per_million"])
            .filter(|n| *n > 0.0)
            .unwrap_or(known_output),
        raw_json: Some(raw),
        ..Default::default()
    })
}

fn string_field<'a>(item: &'a Value, keys: &[&str]) -> &'a str {
    keys.iter()
        .find_map(|k| item.get(k).and_then(Value::as_str))
        .unwrap_or("")
        .trim()
}

fn numeric_field(item: &Value, keys: &[&str]) -> Option<f64> {
    keys.iter().find_map(|k| item.get(k).and_then(Value::as_f64))
}

fn bool_field(item: &Value, fallback: bool, keys: &[&str]) -> bool {
    let includes = |values: &[Value], key: &str| {
        values.iter().filter_map(Value::as_str).any(|s| {
            let s = s.to_lowercase();
            s.contains(key) || s.contains(key.trim_start_matches("supports_"))
        })
    };
    for key in keys {
        match &item[*key] {
            Value::Bool(b) => return *b,
            Value::String(s) => match s.trim().to_lowercase().as_str() {
                "true" | "yes" | "supported" => return true,
                "false" | "no" | "unsupported" => return false,
                _ => {}
            },
            Value::Array(a) if includes(a, key) => return true,
            _ => {}
        }
    }
    item["capabilities"]
        .as_array()
        .is_some_and(|a| keys.iter().any(|k| includes(a, k)))
        || fallback
}

fn records_from_catalog(catalog: &Value) -> Vec<ModelCacheRecord> {
    catalog_items(catalog).iter().filter_map(record).collect()
}

fn offline_records() -> Vec<ModelCacheRecord> {
    ["qwen/qwen3.5-4b-free", "qwen/qwen3.5-9b", "qwen/qwen3.6-35b-a3b", "qwen/qwen3.8-27b", "z-ai/glm-5.3"]
        .iter().filter_map(|id| {
            let (ctx, tools, reasoning, input, output) = known(id);
            let raw = json!({"id": id, "context_window": ctx, "tools": tools, "reasoning": reasoning, "input_price_per_million": json_number(input), "output_price_per_million": json_number(output), "source": "offline_fallback"});
            record(&raw)
        }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slash_ids_are_found_inside_model_list() {
        let list = json!({"object":"list","data":[{"id":"qwen/qwen3.5-9b"},{"id":"z-ai/glm-5.3"}]});
        let found = catalog_items(&list).into_iter().find(|v| v["id"] == "z-ai/glm-5.3");
        assert_eq!(found.unwrap()["id"], "z-ai/glm-5.3");
    }

    #[test]
    fn model_cache_is_seeded_only_when_empty_and_refresh_survives_reopen() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("models.db");
        let mut store = Store::open(&path)?;
        seed_model_cache(&mut store)?;
        assert_eq!(store.all_models()?.len(), 5);
        assert!(store.query_models("fake")?.is_empty());
        refresh_model_cache(&mut store, &json!({"data":[{"id":"fake/model","object":"model"}]}))?;
        refresh_model_cache(&mut store, &json!({"data":[{"id":"qwen/qwen3.5-9b","tools":false}]}))?;
        assert!(refresh_model_cache(&mut store, &json!({"data":[{"id":" "}]})).is_err());
        drop(store);
        let mut store = Store::open(path)?;
        seed_model_cache(&mut store)?;
        assert_eq!(store.all_models()?.len(), 6);
        assert!(store.query_models("qwen3.5-9b tools")?.is_empty());
        let mut rows: Vec<_> = store.query_models("fake")?.iter().map(record_value).collect();
        rows[0]["updated_at"] = "<time>".into();
        assert_eq!(
            crate::output::pretty(&Value::Array(rows))?.replace("\\u003ctime\\u003e", "<time>"),
            include_str!("../../../parity/fixtures/go-output/expected/12.out").replace("exit=0\n", "")
        );
        let separate = Store::open(dir.path().join("separate.db"))?;
        assert!(separate.all_models()?.is_empty());
        Ok(())
    }

    #[test]
    fn catalog_parsing_matches_go_aliases_and_fallbacks() {
        let records = records_from_catalog(&json!({"models":[
            "string/model", 3, null,
            {"id":" ","name":"ignored"},
            {"name":" alias/model ","context_length":128000,"tool_use":"supported","thinking":["thinking"],"prompt_price_per_million":2,"completion_price_per_million":4},
            {"id":"qwen/qwen3.5-9b","context_window":0,"context_length":20,"input_price_per_million":0,"prompt_price_per_million":10,"supports_tools":"no"},
            {"model":"caps","capabilities":["TOOLS","reasoning"],"context_window":1.5,"context_length":100}
        ]}));
        assert_eq!(records.len(), 4);
        assert_eq!(records[0].id, "string/model");
        assert_eq!(
            (
                records[1].context_window,
                records[1].supports_tools,
                records[1].supports_reasoning
            ),
            (128000, true, true)
        );
        assert_eq!(
            (records[1].input_price_per_million, records[1].output_price_per_million),
            (2.0, 4.0)
        );
        assert_eq!(
            (
                records[2].context_window,
                records[2].input_price_per_million,
                records[2].supports_tools
            ),
            (131072, 0.1, false)
        );
        assert!(records[3].supports_tools && records[3].supports_reasoning);
        assert_eq!(records[3].context_window, 0);
    }

    #[test]
    fn live_cache_uses_go_scope_shape_ttl_and_account_isolation() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("live.db");
        let scope = catalog_scope("https://example.com///", "Bearer test");
        assert_eq!(
            scope,
            format!(
                "https://example.com:{}",
                hex::encode(&Sha256::digest(b"Bearer test")[..8])
            )
        );
        let mut store = Store::open(&path)?;
        let rows = parse_live_catalog(&json!({"data":[{"id":"fake/model","object":"model"}]}), "organization");
        let fetched = cached_or_fetch(&mut store, &scope, || Ok((rows.clone(), "organization".into())))?;
        assert_eq!(fetched.0, rows);
        let cached = store.cached_live_catalog(&scope, Duration::from_secs(3600))?.unwrap();
        assert_eq!(
            crate::output::pretty(&cached)?,
            include_str!("../../../parity/fixtures/go-output/expected/16.out").replace("exit=0\n", "")
        );
        drop(store);
        let mut store = Store::open(&path)?;
        assert_eq!(
            cached_or_fetch(&mut store, &scope, || panic!("fresh cache must not fetch"))?,
            fetched
        );
        let other = catalog_scope("https://example.com", "Bearer another");
        assert!(cached_or_fetch(&mut store, &other, || anyhow::bail!("offline")).is_err());
        let other_host = catalog_scope("https://other.example.com", "Bearer test");
        assert!(cached_or_fetch(&mut store, &other_host, || anyhow::bail!("offline")).is_err());
        store
            .connection()
            .execute("UPDATE live_catalog_cache SET fetched_at='2000-01-01T00:00:00Z'", [])?;
        assert!(cached_or_fetch(&mut store, &scope, || anyhow::bail!("offline")).is_err());
        // Go's cache stores an object, not an array; old/wrong shapes refetch.
        store.save_live_catalog(&scope, &json!([]))?;
        assert!(cached_or_fetch(&mut store, &scope, || Ok((vec![], "ids_only".into()))).is_err());
        store.connection().execute(
            "UPDATE live_catalog_cache SET models_json='broken JSON' WHERE scope=?",
            [&scope],
        )?;
        let recovered = cached_or_fetch(&mut store, &scope, || Ok((rows.clone(), "organization".into())))?;
        assert_eq!(recovered.0, rows);
        store.save_live_catalog(&scope, &json!({"models":[{"model_sku":"cached/go","status":"active","price_m_output_tokens_usd":4}],"source":"organization"}))?;
        let (models, source) = cached_or_fetch(&mut store, &scope, || panic!("Go cache should be readable"))?;
        assert_eq!(default_model(&models, &source), "cached/go");
        assert_eq!(models[0]["price_m_output_tokens_usd"].to_string(), "4");
        Ok(())
    }

    #[test]
    fn live_catalog_selects_frontier_and_prices_reported_usage() {
        let models = vec![
            json!({"model_sku":"qwen/free", "status":"active", "price_m_input_tokens_usd":0.0, "price_m_output_tokens_usd":0.0}),
            json!({"model_sku":"z-ai/frontier", "status":"active", "price_m_input_tokens_usd":1.0, "price_m_output_tokens_usd":4.0}),
        ];
        assert_eq!(default_model(&models, "organization"), "z-ai/frontier");
        assert_eq!(
            price(
                "z-ai/frontier",
                &models,
                &json!({"prompt_tokens":1000,"completion_tokens":500})
            ),
            Some(0.003)
        );
        assert_eq!(price("z-ai/frontier", &models, &json!({})), None);
        assert_eq!(price("qwen/free", &models, &json!({})), Some(0.0));
    }
}
