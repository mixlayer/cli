// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

//! JSON output and Go-compatible field projections for `--select` and `--compact`.

use serde_json::Value;

use crate::Globals;

/// Writes bytes to stdout. A closed pipe (`mixlayer ... | head`) is not an error.
pub fn write_stdout(b: &[u8]) -> anyhow::Result<()> {
    use std::io::Write;
    let mut out = std::io::stdout().lock();
    match out.write_all(b).and_then(|_| out.flush()) {
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        r => Ok(r?),
    }
}

/// Makes `println!` exit quietly instead of panicking when stdout is a
/// closed pipe, matching how the Go CLI behaves under `| head`.
pub fn quiet_broken_pipe() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let msg = info
            .payload()
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| info.payload().downcast_ref::<&str>().copied())
            .unwrap_or("");
        if msg.contains("Broken pipe") || msg.contains("failed printing to stdout") {
            std::process::exit(0);
        }
        default(info);
    }));
}

pub fn print_json(v: &Value, g: &Globals) -> anyhow::Result<()> {
    let v = filtered(v.clone(), g);
    let mut s = serde_json::to_string_pretty(&v)?;
    s.push('\n');
    write_stdout(s.as_bytes())
}

pub fn filtered(v: Value, g: &Globals) -> Value {
    match &g.select {
        Some(sel) if !sel.trim().is_empty() => select(&v, sel),
        _ if g.compact => compact(v),
        _ => v,
    }
}

fn compact(v: Value) -> Value {
    match v {
        Value::Array(items) if items.iter().all(Value::is_object) => {
            let n = items.len();
            let mut keep: std::collections::HashSet<String> = [
                "id",
                "name",
                "title",
                "identifier",
                "code",
                "slug",
                "key",
                "status",
                "state",
                "type",
                "kind",
                "priority",
                "url",
                "email",
                "price",
                "amount",
                "cost",
                "fare",
                "rate",
                "currency",
                "rating",
                "score",
                "count",
                "language",
                "locale",
                "country",
                "region",
                "city",
                "domain",
                "created_at",
                "updated_at",
                "createdAt",
                "updatedAt",
                "date",
                "version",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect();
            let mut counts = std::collections::HashMap::<String, usize>::new();
            for obj in items.iter().filter_map(Value::as_object) {
                for key in obj.keys() {
                    if ![
                        "description",
                        "body",
                        "content",
                        "comments",
                        "attachments",
                        "html",
                        "markdown",
                    ]
                    .contains(&key.as_str())
                    {
                        *counts.entry(key.clone()).or_default() += 1;
                    }
                }
            }
            let threshold = if n >= 2 {
                (n * 4).div_ceil(5).min(n - 1)
            } else {
                (n * 4).div_ceil(5)
            };
            keep.extend(
                counts
                    .into_iter()
                    .filter_map(|(key, count)| (count >= threshold).then_some(key)),
            );
            Value::Array(
                items
                    .into_iter()
                    .map(|item| {
                        let Value::Object(mut obj) = item else { unreachable!() };
                        let original = obj.clone();
                        obj.retain(|key, _| keep.contains(key));
                        Value::Object(if obj.is_empty() { original } else { obj })
                    })
                    .collect(),
            )
        }
        Value::Array(items) => Value::Array(items),
        Value::Object(mut map) => {
            for key in ["description", "comments", "attachments"] {
                map.remove(key);
            }
            Value::Object(map)
        }
        other => other,
    }
}

fn select(v: &Value, sel: &str) -> Value {
    let paths: Vec<Vec<String>> = sel
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.split('.').map(str::to_ascii_lowercase).collect())
        .collect();
    select_paths(v, &paths)
}

fn select_paths(v: &Value, paths: &[Vec<String>]) -> Value {
    match v {
        Value::Array(items) => Value::Array(items.iter().map(|v| select_paths(v, paths)).collect()),
        Value::Object(obj) => {
            let mut out = serde_json::Map::new();
            for (key, value) in obj {
                let lower = key.to_ascii_lowercase();
                let kebab = camel_to_kebab(key);
                let relevant: Vec<_> = paths
                    .iter()
                    .filter(|p| p.first().is_some_and(|part| part == &lower || part == &kebab))
                    .collect();
                if relevant.iter().any(|p| p.len() == 1) {
                    out.insert(key.clone(), value.clone());
                } else if !relevant.is_empty() {
                    let tails: Vec<Vec<String>> = relevant.iter().map(|p| p[1..].to_vec()).collect();
                    out.insert(key.clone(), select_paths(value, &tails));
                }
            }
            if out.is_empty() && obj.values().any(Value::is_array) {
                for (key, value) in obj {
                    out.insert(
                        key.clone(),
                        if value.is_array() {
                            select_paths(value, paths)
                        } else {
                            value.clone()
                        },
                    );
                }
            }
            Value::Object(out)
        }
        other => other.clone(),
    }
}

fn camel_to_kebab(name: &str) -> String {
    let mut out = String::new();
    let mut previous_lower = false;
    for c in name.chars() {
        if c.is_uppercase() && previous_lower {
            out.push('-');
        }
        previous_lower = c.is_lowercase();
        out.extend(c.to_lowercase());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn select_descends_arrays_and_keeps_envelope_metadata() {
        let value = serde_json::json!({"total_count": 1, "data": [{"orderDate": "today", "secret": "hidden"}]});
        assert_eq!(
            select(&value, "order-date"),
            serde_json::json!({"total_count": 1, "data": [{"orderDate": "today"}]})
        );
    }

    #[test]
    fn compact_preserves_identity_and_strips_verbose_list_fields() {
        let value = serde_json::json!([{"id":"a", "description":"long"}, {"id":"b", "description":"long"}]);
        assert_eq!(compact(value), serde_json::json!([{"id":"a"}, {"id":"b"}]));
    }
}
