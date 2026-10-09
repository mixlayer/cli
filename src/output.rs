// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

//! JSON output and Go-compatible field projections for `--select` and `--compact`.

use serde_json::Value;

use crate::Globals;

mod table;
pub use table::table;
#[cfg(test)]
mod parity_tests;

#[derive(Debug, Default, PartialEq)]
pub struct Rendered {
    pub stdout: String,
    pub stderr: String,
}

pub fn wants_human_table(g: &Globals, terminal: bool) -> bool {
    terminal && !g.json && !g.csv && !g.plain && !g.quiet && !g.compact && g.select.as_deref().is_none_or(str::is_empty)
}

/// Endpoint commands filter the payload before adding provenance. Explicit
/// JSON wins over quiet/CSV; those flags only opt out of automatic pipe JSON.
pub fn format_endpoint(
    raw: &str,
    g: &Globals,
    terminal: bool,
    single_count: usize,
    color: bool,
) -> anyhow::Result<Rendered> {
    let value: Value = serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.into()));
    let mut result = Rendered::default();
    if wants_human_table(g, terminal) {
        let count = value.as_array().map_or(single_count, Vec::len);
        result.stderr = format!("{count} results (live)\n");
    }
    if g.json || (!terminal && !g.csv && !g.quiet && !g.plain) {
        let mut data = filtered(value, g);
        if let Some(obj) = data.as_object()
            && obj.len() == 1
            && let Some(array) = ["results", "data", "items", "nodes", "entries", "records"]
                .iter()
                .find_map(|k| obj.get(*k))
            && array.is_array()
        {
            data = array.clone();
        }
        result.stdout = pretty(&serde_json::json!({"meta": {"source": "live"}, "results": data}))?;
    } else {
        let rendered = format_output(raw, g, terminal, color)?;
        result.stdout = rendered.stdout;
        result.stderr.push_str(&rendered.stderr);
    }
    Ok(result)
}

pub fn print_endpoint(raw: &str, g: &Globals, single_count: usize) -> anyhow::Result<()> {
    let terminal = stdout_is_terminal();
    let color = terminal
        && !g.no_color
        && !crate::agent_detected()
        && std::env::var("NO_COLOR").unwrap_or_default().is_empty()
        && std::env::var("TERM").as_deref() != Ok("dumb");
    let rendered = format_endpoint(raw, g, terminal, single_count, color)?;
    eprint!("{}", rendered.stderr);
    write_stdout(rendered.stdout.as_bytes())
}

/// Go checks the file's character-device bit (including /dev/null), rather
/// than asking whether it supports terminal ioctls.
pub fn stdout_is_terminal() -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileTypeExt;
        std::fs::metadata("/dev/fd/1").map_or(true, |m| m.file_type().is_char_device())
    }
    #[cfg(not(unix))]
    {
        use std::io::IsTerminal;
        std::io::stdout().is_terminal()
    }
}

pub fn format_output(raw: &str, g: &Globals, terminal: bool, color: bool) -> anyhow::Result<Rendered> {
    let mut out = Rendered::default();
    if g.quiet {
        return Ok(out);
    }
    let parsed = serde_json::from_str::<Value>(raw).ok();
    let projected = match g.select.as_deref().filter(|s| !s.is_empty()) {
        Some(select) => select.split(',').any(|s| !s.trim().is_empty()),
        None => g.compact,
    };
    let value = filtered(parsed.clone().unwrap_or(Value::Null), g);
    let raw = if projected && parsed.is_some() {
        escape_json_html(serde_json::to_string(&value)?)
    } else {
        raw.to_owned()
    };
    if g.csv {
        out.stdout = csv_output(&value, &raw);
    } else if g.json || !terminal {
        out.stdout = if parsed.is_some() {
            pretty(&value)?
        } else {
            format!("{raw}\n")
        };
    } else if let Some(items) = value
        .as_array()
        .filter(|a| !a.is_empty() && a.iter().all(Value::is_object))
    {
        out.stdout = table::auto_table(items, color);
        if items.len() >= 25 {
            out.stderr = format!(
                "\nShowing {} results. To narrow: add --limit, --json --select, or filter flags.\n",
                items.len()
            );
        }
    } else if value.is_object() {
        let mut sorted = value;
        sorted.sort_all_objects();
        out.stdout = pretty(&sorted)?;
    } else {
        out.stdout = format!("{raw}\n");
    }
    Ok(out)
}

pub(crate) fn pretty(value: &Value) -> anyhow::Result<String> {
    Ok(format!("{}\n", escape_json_html(serde_json::to_string_pretty(value)?)))
}

fn escape_json_html(raw: String) -> String {
    // encoding/json's default escaping also applies when embedding RawMessage.
    raw.replace('&', "\\u0026")
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

fn csv_output(value: &Value, raw: &str) -> String {
    let Some(items) = value
        .as_array()
        .filter(|a| !a.is_empty() && a.iter().all(Value::is_object))
    else {
        return format!("{raw}\n");
    };
    let keys: std::collections::BTreeSet<_> = items.iter().flat_map(|v| v.as_object().unwrap().keys()).collect();
    let mut out = keys.iter().map(|k| k.as_str()).collect::<Vec<_>>().join(",");
    out.push('\n');
    for item in items {
        let cells: Vec<_> = keys
            .iter()
            .map(|key| {
                let v = &item[*key];
                let s = if v.is_null() { String::new() } else { go_display(v) };
                if s.contains([',', '"', '\n']) {
                    format!("\"{}\"", s.replace('"', "\"\""))
                } else {
                    s
                }
            })
            .collect();
        out.push_str(&cells.join(","));
        out.push('\n');
    }
    out
}

fn go_display(v: &Value) -> String {
    match v {
        Value::Null => "<nil>".into(),
        Value::String(s) => s.clone(),
        Value::Number(n) => n.as_f64().map(|f| f.to_string()).unwrap_or_else(|| n.to_string()),
        Value::Array(a) => format!("[{}]", a.iter().map(go_display).collect::<Vec<_>>().join(" ")),
        Value::Object(m) => {
            let sorted: std::collections::BTreeMap<_, _> = m.iter().collect();
            format!(
                "map[{}]",
                sorted
                    .into_iter()
                    .map(|(k, v)| format!("{k}:{}", go_display(v)))
                    .collect::<Vec<_>>()
                    .join(" ")
            )
        }
        Value::Bool(b) => b.to_string(),
    }
}

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
        _ if g.select.as_deref().is_none_or(str::is_empty) && g.compact => {
            let mut v = compact(v);
            v.sort_all_objects();
            v
        }
        _ => v,
    }
}

fn compact(v: Value) -> Value {
    match v {
        Value::Null => Value::Array(Vec::new()),
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
    if paths.is_empty() {
        return v.clone();
    }
    select_paths(v, &paths)
}

fn select_paths(v: &Value, paths: &[Vec<String>]) -> Value {
    match v {
        Value::Null => Value::Array(Vec::new()),
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
            out.sort_keys();
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
