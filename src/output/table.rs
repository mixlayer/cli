use serde_json::{Map, Value};

/// Go's tabwriter padding is two spaces; the final column is unpadded.
/// Explicit table callers intentionally ignore CSV/plain/quiet flags.
pub fn table(rows: &[Vec<String>]) -> String {
    let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
    let widths: Vec<usize> = (0..cols)
        .map(|i| {
            rows.iter()
                .filter_map(|r| r.get(i))
                .map(|c| c.chars().count())
                .max()
                .unwrap_or(0)
        })
        .collect();
    let mut out = String::new();
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            out.push_str(cell);
            if i + 1 < row.len() {
                out.push_str(&" ".repeat(widths[i] + 2 - cell.chars().count()));
            }
        }
        out.push('\n');
    }
    out
}

fn bold(s: &str, color: bool) -> String {
    if color { format!("\x1b[1m{s}\x1b[0m") } else { s.into() }
}

fn priority(key: &str, value: &Value) -> usize {
    let lower = key.to_lowercase();
    let exact = match lower.as_str() {
        "id" | "name" | "title" | "slug" | "key" => Some(0),
        "date" | "created" | "updated" | "createdat" | "updatedat" => Some(1),
        "status" | "state" | "statuscode" => Some(2),
        "summary" | "description" | "price" | "amount" | "total" | "cost" | "points" | "score" => Some(3),
        "type" | "kind" | "category" | "email" | "phone" | "url" => Some(4),
        _ => None,
    };
    if let Some(tier) = exact {
        return tier;
    }
    // Go lowercases before splitting, so only underscores/hyphens split here.
    let parts: Vec<_> = lower.split(['_', '-']).filter(|s| !s.is_empty()).collect();
    let tier = match parts.last().copied().unwrap_or("") {
        "id" | "name" | "title" => 0,
        "date" | "time" => 1,
        "status" | "state" | "code" => 2,
        "price" | "amount" | "total" | "cost" | "summary" | "description" | "points" | "score" => 3,
        "type" | "kind" | "category" | "method" => 4,
        _ => return if value.is_boolean() { 6 } else { 5 },
    };
    tier + usize::from(parts.len() > 1)
}

pub(super) fn auto_table(items: &[Value], color: bool) -> String {
    let first = items[0].as_object().unwrap();
    let complex = first.values().filter(|v| v.is_array() || v.is_object()).count();
    let cards = first.len() > 8 || complex > 2;
    let mut headers: Vec<_> = first
        .iter()
        .filter(|(_, v)| {
            if cards {
                !cell(v).is_empty()
            } else {
                !v.is_array() && !v.is_object()
            }
        })
        .map(|(k, v)| (k, priority(k, v)))
        .collect();
    headers.sort_by_key(|(_, tier)| *tier);
    let mut headers: Vec<_> = headers.into_iter().map(|(k, _)| k).collect();
    if cards {
        return auto_cards(items, &headers, color);
    }
    headers.truncate(6);
    let mut rows = vec![headers.iter().map(|h| h.to_uppercase()).collect()];
    rows.extend(
        items
            .iter()
            .map(|item| headers.iter().map(|h| cell(&item[*h])).collect()),
    );
    let rendered = table(&rows);
    if !color {
        return rendered;
    }
    let (header, rest) = rendered.split_once('\n').unwrap_or((&rendered, ""));
    let mut header = header.to_owned();
    for h in &headers {
        let upper = h.to_uppercase();
        header = header.replacen(&upper, &bold(&upper, color), 1);
    }
    format!("{header}\n{rest}")
}

fn auto_cards(items: &[Value], headers: &[&String], color: bool) -> String {
    let Some(first) = headers.first() else {
        return String::new();
    };
    let max = headers.iter().map(|h| h.len()).max().unwrap_or(0);
    let mut out = String::new();
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&format!(
            "{} {}",
            bold(&first.to_uppercase(), color),
            cell(&item[*first])
        ));
        if let Some(second) = headers.get(1) {
            let value = cell(&item[*second]);
            if !value.is_empty() {
                out.push_str(&format!(" — {value}"));
            }
        }
        out.push('\n');
        for h in headers.iter().skip(2) {
            let value = cell(&item[*h]);
            if ["", "false", "0", "[]", "null"].contains(&value.as_str()) {
                continue;
            }
            if value.starts_with('\n') {
                out.push_str(&format!("  {h}:{value}\n"));
            } else {
                out.push_str(&format!("  {:<max$}  {value}\n", format!("{h}:")));
            }
        }
    }
    out
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.into();
    }
    let mut end = max - 3;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &s[..end])
}

fn find(obj: &Map<String, Value>, names: &[&str]) -> String {
    names
        .iter()
        .find_map(|name| {
            obj.iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .map(|(_, v)| cell(v))
        })
        .unwrap_or_default()
}

fn object_summary(obj: &Map<String, Value>) -> String {
    let mut parts = Vec::new();
    let qty = find(obj, &["qty", "count", "quantity"]);
    if !qty.is_empty() && qty != "0" {
        parts.push(format!("{qty}x"));
    }
    let mut name = find(obj, &["name", "title", "label", "description"]);
    if name.is_empty() {
        for key in ["Side1", "side1", "Item", "item", "Product", "product"] {
            if let Some(nested) = obj.get(key).and_then(Value::as_object) {
                name = find(nested, &["name", "title", "label"]);
                if !name.is_empty() {
                    break;
                }
            }
        }
    }
    if !name.is_empty() {
        parts.push(name);
    }
    let mut size = find(obj, &["sizename", "size_name"]);
    if size.is_empty() {
        size = find(obj, &["catname", "cat_name", "category"]);
    }
    if !size.is_empty() {
        parts.push(format!("— {size}"));
    }
    let price = find(obj, &["extprice", "price", "amount", "total"]);
    if !price.is_empty() && price != "0" {
        parts.push(format!("(${price})"));
    }
    if parts.is_empty() {
        let mut value = Value::Object(obj.clone());
        value.sort_all_objects();
        truncate(&value.to_string(), 80)
    } else {
        format!("    {}", parts.join(" "))
    }
}

fn cell(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => {
            let b = s.as_bytes();
            if b.len() >= 19 && b[4] == b'-' && b[7] == b'-' && b[10] == b'T' {
                s.get(..10).unwrap_or(s).into()
            } else {
                truncate(s, 60)
            }
        }
        Value::Number(n) => {
            let f = n.as_f64().unwrap_or(0.0);
            if f.fract() == 0.0 {
                format!("{f:.0}")
            } else {
                format!("{f:.2}")
            }
        }
        Value::Bool(b) => b.to_string(),
        Value::Array(a) if a.first().is_some_and(Value::is_object) => {
            format!(
                "\n{}",
                a.iter()
                    .filter_map(Value::as_object)
                    .map(object_summary)
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        }
        Value::Array(a) => truncate(
            &a.iter()
                .map(|v| v.as_str().map(str::to_owned).unwrap_or_else(|| v.to_string()))
                .collect::<Vec<_>>()
                .join(", "),
            60,
        ),
        Value::Object(obj) => find(obj, &["name", "title", "label", "description", "id", "key", "code"]),
    }
}
