use anyhow::{Result, anyhow};

use super::DictionaryTerm;

pub fn column_kind(col: &str) -> String {
    let mut out = String::new();
    let mut underscore = true;
    for c in col.to_uppercase().chars() {
        if c.is_ascii_uppercase() {
            out.push(c);
            underscore = false;
        } else if !underscore {
            out.push('_');
            underscore = true;
        }
    }
    let kind = out.trim_matches('_');
    if kind.is_empty() { "FIELD".into() } else { kind.into() }
}

pub fn csv_column_terms(data: &str, columns: &[&str]) -> Result<Vec<DictionaryTerm>> {
    if columns.is_empty() {
        return Ok(Vec::new());
    }
    if data.trim().is_empty() {
        return Err(anyhow!("--mask-columns needs CSV data (pass a file with --data)"));
    }
    let rows = read_lazy_csv(data);
    let Some(header) = rows.first() else {
        return Err(anyhow!("--mask-columns: CSV has no header row"));
    };
    let mut indexes = std::collections::HashMap::new();
    for (i, h) in header.iter().enumerate() {
        indexes.insert(h.trim_start_matches('\u{feff}').trim().to_lowercase(), i);
    }
    let mut terms = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for col in columns {
        let col = col.trim();
        if col.is_empty() {
            continue;
        }
        let Some(&idx) = indexes.get(&col.to_lowercase()) else {
            return Err(anyhow!(
                "--mask-columns: no column {:?}; the CSV has: {}",
                col,
                header.join(", ")
            ));
        };
        for row in &rows[1..] {
            let Some(v) = row.get(idx) else {
                continue;
            };
            let v = v.trim();
            if v.chars().count() < 2 || !seen.insert(v.to_lowercase()) {
                continue;
            }
            terms.push(DictionaryTerm {
                term: v.into(),
                kind: column_kind(col),
            });
        }
    }
    Ok(terms)
}

// Go's csv.Reader uses both LazyQuotes and TrimLeadingSpace here. The csv crate
// has no LazyQuotes mode, so parse this one input shape directly.
fn read_lazy_csv(data: &str) -> Vec<Vec<String>> {
    let normalized = data.replace("\r\n", "\n");
    let bytes = normalized.as_bytes();
    let mut rows = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\n' {
            i += 1;
            continue;
        }
        let mut row = Vec::new();
        loop {
            while i < bytes.len() && bytes[i] != b'\n' {
                let c = normalized[i..].chars().next().unwrap();
                if !c.is_whitespace() {
                    break;
                }
                i += c.len_utf8();
            }
            let mut field = String::new();
            let quoted = i < bytes.len() && bytes[i] == b'"';
            if quoted {
                i += 1;
            }
            loop {
                if i == bytes.len() {
                    break;
                }
                let b = bytes[i];
                if quoted && b == b'"' {
                    match bytes.get(i + 1) {
                        Some(b'"') => {
                            field.push('"');
                            i += 2;
                            continue;
                        }
                        Some(b',') | Some(b'\n') | None => {
                            i += 1;
                            break;
                        }
                        _ => {
                            field.push('"');
                            i += 1;
                            continue;
                        }
                    }
                }
                if !quoted && (b == b',' || b == b'\n') {
                    break;
                }
                let c = normalized[i..].chars().next().unwrap();
                field.push(c);
                i += c.len_utf8();
            }
            row.push(field);
            if i == bytes.len() {
                break;
            }
            if bytes[i] == b',' {
                i += 1;
                continue;
            }
            if bytes[i] == b'\n' {
                i += 1;
                break;
            }
            // A closing quote followed by a comma or newline has already
            // consumed the quote. Any other character is a lazy bare quote.
        }
        rows.push(row);
    }
    rows
}
