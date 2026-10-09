use anyhow::Result;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tranche {
    pub index: usize,
    pub from_line: usize,
    pub to_line: usize,
    pub text: String,
}

fn csv_text(header: &csv::StringRecord, rows: &[csv::StringRecord]) -> String {
    let mut w = csv::WriterBuilder::new().has_headers(false).from_writer(Vec::new());
    w.write_record(header).unwrap();
    for row in rows {
        w.write_record(row).unwrap();
    }
    String::from_utf8(w.into_inner().unwrap()).unwrap()
}

pub fn split_records(input: &str, max_bytes: usize) -> Result<Vec<Tranche>> {
    let max_bytes = if max_bytes == 0 { 64 * 1024 } else { max_bytes };
    let records = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .from_reader(input.as_bytes())
        .records()
        .collect::<Result<Vec<_>, _>>();
    if let Ok(records) = records
        && records.first().is_some_and(|r| r.len() >= 2)
    {
        let mut out = Vec::new();
        let mut batch = Vec::new();
        let mut from = 2;
        for (i, row) in records.iter().enumerate().skip(1) {
            batch.push(row.clone());
            if batch.len() > 1 && csv_text(&records[0], &batch).len() > max_bytes {
                let last = batch.pop().unwrap();
                out.push(Tranche {
                    index: out.len() + 1,
                    from_line: from,
                    to_line: i,
                    text: csv_text(&records[0], &batch),
                });
                batch = vec![last];
                from = i + 1;
            }
        }
        if !batch.is_empty() {
            out.push(Tranche {
                index: out.len() + 1,
                from_line: from,
                to_line: records.len(),
                text: csv_text(&records[0], &batch),
            });
        }
        return Ok(out);
    }
    let mut out = Vec::new();
    let mut buffer = String::new();
    let mut from = 1;
    let mut line = 0;
    for raw in input.split_terminator('\n') {
        line += 1;
        let next = format!("{}\n", raw.trim_end_matches('\r'));
        if buffer.len() + next.len() > max_bytes && !buffer.is_empty() {
            out.push(Tranche {
                index: out.len() + 1,
                from_line: from,
                to_line: line - 1,
                text: std::mem::take(&mut buffer),
            });
            from = line;
        }
        if next.len() > max_bytes {
            for part in split_oversized(&next, max_bytes) {
                out.push(Tranche {
                    index: out.len() + 1,
                    from_line: line,
                    to_line: line,
                    text: part,
                });
            }
            from = line + 1;
        } else {
            buffer.push_str(&next);
        }
    }
    if !buffer.is_empty() {
        out.push(Tranche {
            index: out.len() + 1,
            from_line: from,
            to_line: line,
            text: buffer,
        });
    }
    if out.is_empty() {
        out.push(Tranche {
            index: 1,
            from_line: 1,
            to_line: 1,
            text: input.into(),
        });
    }
    Ok(out)
}

pub fn split_oversized(s: &str, max_bytes: usize) -> Vec<String> {
    if max_bytes == 0 || s.len() <= max_bytes {
        return vec![s.into()];
    }
    let mut out = Vec::new();
    let mut start = 0;
    while start < s.len() {
        let mut end = (start + max_bytes).min(s.len());
        while end > start && !s.is_char_boundary(end) {
            end -= 1;
        }
        if end == start {
            end = start + s[start..].chars().next().unwrap().len_utf8();
        }
        if end < s.len()
            && let Some(dot) = s[start..end].rfind(['.', '!', '?', ';'])
            && dot > max_bytes / 2
        {
            end = start + dot + 1;
        }
        out.push(s[start..end].into());
        start = end;
    }
    out
}
