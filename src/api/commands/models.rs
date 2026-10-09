use std::time::Duration;

use clap::ArgMatches;
use serde_json::{Value, json};

use super::{as_results, bool_arg, classified, new_client, print, words};
use crate::Globals;
use crate::api::client::{ApiError, Client, parse_duration};
use crate::exit::{self, Code};

fn catalog_items(value: &Value) -> Vec<Value> {
    if let Some(array) = value.as_array() {
        return array.clone();
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

fn human(value: &Value) {
    if let Some(rows) = value.as_array().or_else(|| value.get("data").and_then(Value::as_array)) {
        for row in rows {
            if let Some(id) = row.get("id").and_then(Value::as_str) {
                println!("{id}");
            } else {
                println!("{row}");
            }
        }
    } else {
        println!("{value}");
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
    let data = client.get("/models").map_err(|e| classified(e, g, "models-list"))?;
    if g.json {
        print(&as_results(data, g), &Globals::default())
    } else {
        human(&data);
        Ok(())
    }
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
    let data = client.get("/models").map_err(|e| classified(e, g, "models-get"))?;
    let item = catalog_items(&data)
        .into_iter()
        .find(|v| v.get("id").and_then(Value::as_str) == Some(id));
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
    if g.json {
        print(&as_results(item, g), &Globals::default())
    } else {
        human(&item);
        Ok(())
    }
}

fn live_prices(m: &ArgMatches, g: &Globals) -> anyhow::Result<()> {
    if g.dry_run {
        return print(
            &json!({"endpoint": "/v1/organizations/{org}/models", "fallback_endpoint": "/v1/models", "dry_run": true}),
            g,
        );
    }
    let (models, source, warning) = match load_live_catalog(m) {
        Ok((models, source)) => (models, source, None),
        Err(e) => (offline_live_models(), "offline_fallback".into(), Some(e.to_string())),
    };
    let mut result = json!({"models": models, "source": source});
    if let Some(warning) = warning.as_ref() {
        result["warning"] = warning.as_str().into();
    }
    if g.json {
        print(&result, g)
    } else {
        if let Some(warning) = warning {
            eprintln!("warning: {warning}");
        }
        for model in models {
            println!(
                "{}\t{}",
                model["model_sku"].as_str().unwrap_or(""),
                model["status"].as_str().unwrap_or("")
            );
        }
        Ok(())
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
    let base = std::env::var("MIXLAYER_PLATFORM_URL").unwrap_or_else(|_| "https://api.mixlayer.com".into());
    let url = url::Url::parse(&base).map_err(|_| exit::err(Code::Config, "invalid MIXLAYER_PLATFORM_URL"))?;
    if !["http", "https"].contains(&url.scheme()) || url.host().is_none() || url.username() != "" {
        return Err(exit::err(Code::Config, "invalid MIXLAYER_PLATFORM_URL"));
    }
    // Platform prices use a separate origin and a five-second probe timeout.
    client.config.base_url = base.trim_end_matches('/').into();
    let organizations = client.probe("/v1/organizations");
    let mut source = "ids_only";
    let mut data = None;
    if let Ok(orgs) = organizations {
        for org in catalog_items(&orgs) {
            if let Some(id) = ["id", "slug", "organization_id"]
                .iter()
                .find_map(|k| org.get(k).and_then(Value::as_str))
                && let Ok(prices) = client.probe(&format!(
                    "/v1/organizations/{}/models",
                    url::form_urlencoded::byte_serialize(id.as_bytes()).collect::<String>()
                ))
                && !catalog_items(&prices).is_empty()
            {
                data = Some(prices);
                source = "organization";
                break;
            }
        }
    }
    if data.is_none() {
        data = Some(client.probe("/v1/models")?);
    }
    let models: Vec<Value> = catalog_items(&data.expect("set"))
        .iter().filter_map(|item| {
            let sku = ["model_sku", "id", "model", "name"].iter().find_map(|k| item.get(k).and_then(Value::as_str))?;
            let mut model = json!({
                "model_sku": sku,
                "price_m_input_tokens_usd": if source == "organization" { number(item.get("price_m_input_tokens_usd")) } else { None },
                "price_m_output_tokens_usd": if source == "organization" { number(item.get("price_m_output_tokens_usd")) } else { None },
                "price_m_input_cache_read_tokens_usd": if source == "organization" { number(item.get("price_m_input_cache_read_tokens_usd")) } else { None },
                "max_context_len": item.get("max_context_len").or_else(|| item.get("context_window")).and_then(Value::as_i64).unwrap_or(0),
                "tools_supported": item.get("tools_supported").and_then(Value::as_bool).unwrap_or(false),
                "input_modalities": item.get("input_modalities").cloned().unwrap_or(Value::Null),
                "status": item.get("status").and_then(Value::as_str).unwrap_or(if source == "ids_only" { "unknown" } else { "" }),
                "access": item.get("access"),
            });
            if let Some(model_type) = item.get("model_type").or_else(|| item.get("type")).and_then(Value::as_str).filter(|s| !s.is_empty()) {
                model["model_type"] = model_type.into();
            }
            Some(model)
        }).collect();
    Ok((models, source.into()))
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

fn context_number(raw: &str) -> i64 {
    let lower = raw.trim().to_ascii_lowercase();
    let num = lower.trim_end_matches('b').trim_end_matches('k');
    num.parse::<i64>().unwrap_or(0)
        * if lower.ends_with('k') || lower.ends_with("kb") {
            1000
        } else {
            1
        }
}

fn matches_query(record: &Value, dsl: &str) -> bool {
    let id = record["id"].as_str().unwrap_or("").to_ascii_lowercase();
    let ctx = record["context_window"].as_i64().unwrap_or(0);
    dsl.to_ascii_lowercase().split_whitespace().all(|term| {
        if term == "tools" {
            record["supports_tools"] == true
        } else if term == "reasoning" {
            record["supports_reasoning"] == true
        } else if let Some(v) = term.strip_prefix("ctx>=") {
            ctx >= context_number(v)
        } else if let Some(v) = term.strip_prefix("ctx>") {
            ctx > context_number(v)
        } else if let Some(v) = term.strip_prefix("ctx=") {
            ctx == context_number(v)
        } else {
            id.contains(term)
        }
    })
}

pub fn query(m: &ArgMatches, g: &Globals) -> anyhow::Result<()> {
    let args = words(m, "models query");
    let Some(dsl) = args.first() else {
        return Err(exit::usage("query DSL is required"));
    };
    if g.dry_run {
        return print(&json!({"dry_run": true, "endpoint": "/models", "query": dsl}), g);
    }
    let records = if bool_arg(m, "refresh") {
        let mut client = new_client(m)?;
        let catalog = client
            .get_no_cache("/models")
            .map_err(|e| classified(e, g, "models-query"))?;
        records_from_catalog(&catalog)
    } else {
        offline_records()
    };
    let mut records: Vec<Value> = records.into_iter().filter(|r| matches_query(r, dsl)).collect();
    records.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
    print(&Value::Array(records), &Globals::default())
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

fn record(item: &Value) -> Option<Value> {
    let id = ["id", "name", "model"]
        .iter()
        .find_map(|k| item.get(k).and_then(Value::as_str))?;
    let (known_ctx, known_tools, known_reasoning, known_input, known_output) = known(id);
    let ctx = ["context_window", "context_length", "context_size", "max_context_tokens"]
        .iter()
        .find_map(|k| item.get(k).and_then(Value::as_i64))
        .filter(|n| *n > 0)
        .unwrap_or(known_ctx);
    Some(json!({
        "id": id, "context_window": ctx,
        "supports_tools": item.get("supports_tools").or_else(|| item.get("tools")).and_then(Value::as_bool).unwrap_or(known_tools),
        "supports_reasoning": item.get("supports_reasoning").or_else(|| item.get("reasoning")).and_then(Value::as_bool).unwrap_or(known_reasoning),
        "input_price_per_million": item.get("input_price_per_million").and_then(Value::as_f64).filter(|n| *n > 0.0).unwrap_or(known_input),
        "output_price_per_million": item.get("output_price_per_million").and_then(Value::as_f64).filter(|n| *n > 0.0).unwrap_or(known_output),
        "raw_json": item,
        "updated_at": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    }))
}

fn records_from_catalog(catalog: &Value) -> Vec<Value> {
    catalog_items(catalog).iter().filter_map(record).collect()
}

fn offline_records() -> Vec<Value> {
    ["qwen/qwen3.5-4b-free", "qwen/qwen3.5-9b", "qwen/qwen3.6-35b-a3b", "qwen/qwen3.8-27b", "z-ai/glm-5.3"]
        .iter().filter_map(|id| {
            let (ctx, tools, reasoning, input, output) = known(id);
            let raw = json!({"id": id, "context_window": ctx, "tools": tools, "reasoning": reasoning, "input_price_per_million": input, "output_price_per_million": output, "source": "offline_fallback"});
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
    fn model_query_terms_match_go() {
        let record =
            json!({"id":"qwen/large", "context_window": 128000, "supports_tools": true, "supports_reasoning": false});
        assert!(matches_query(&record, "ctx>=128k tools qwen"));
        assert!(!matches_query(&record, "reasoning"));
        assert!(!matches_query(&record, "ctx>128k"));
        assert_eq!(offline_records().len(), 5);
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
