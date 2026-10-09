use anyhow::Result;
use regex::Regex;

#[derive(Default)]
pub struct RestructureOptions {
    pub bucket_numerics: bool,
    pub coarsen_dates: String,
    pub drop_columns: Vec<String>,
}

pub fn restructure(input: &str, opts: &RestructureOptions) -> Result<String> {
    let rows = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .from_reader(input.as_bytes())
        .records()
        .collect::<Result<Vec<_>, _>>();
    if let Ok(rows) = rows
        && rows.first().is_some_and(|r| r.len() > 1)
    {
        let header = &rows[0];
        let drops: Vec<bool> = header
            .iter()
            .map(|h| {
                opts.drop_columns
                    .iter()
                    .any(|d| d.trim().eq_ignore_ascii_case(h.trim()))
            })
            .collect();
        let mut writer = csv::WriterBuilder::new().has_headers(false).from_writer(Vec::new());
        for row in rows {
            let output: Vec<_> = row
                .iter()
                .enumerate()
                .filter(|(i, _)| !drops.get(*i).copied().unwrap_or(false))
                .map(|(_, cell)| coarsen_cell(cell, opts))
                .collect();
            writer.write_record(output)?;
        }
        return Ok(String::from_utf8(writer.into_inner()?)?);
    }
    Ok(input
        .split('\n')
        .map(|line| coarsen_cell(line, opts))
        .collect::<Vec<_>>()
        .join("\n"))
}

fn coarsen_cell(cell: &str, opts: &RestructureOptions) -> String {
    let mut out = cell.to_string();
    if !opts.coarsen_dates.is_empty() {
        let re = Regex::new(r"\b[0-9]{4}-[0-9]{2}-[0-9]{2}\b").unwrap();
        out = re
            .replace_all(&out, |caps: &regex::Captures| {
                let s = &caps[0];
                let (Ok(year), Ok(month), Ok(day)) =
                    (s[..4].parse::<i32>(), s[5..7].parse::<u32>(), s[8..10].parse::<u32>())
                else {
                    return s.to_string();
                };
                let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
                let days = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
                if month == 0 || month > 12 || day == 0 || day > days[(month.saturating_sub(1) as usize).min(11)] {
                    return s.to_string();
                }
                if ["quarter", "quarters"].contains(&opts.coarsen_dates.to_lowercase().as_str()) {
                    format!("{year:04}-Q{}", (month - 1) / 3 + 1)
                } else {
                    format!("{year:04}-{month:02}")
                }
            })
            .to_string();
    }
    if opts.bucket_numerics && Regex::new(r"^\s*[0-9]+(?:\.[0-9]+)?\s*$").unwrap().is_match(&out) {
        let re = Regex::new(r"\b[0-9]+(?:\.[0-9]+)?\b").unwrap();
        out = re
            .replace_all(&out, |caps: &regex::Captures| {
                let n = caps[0].parse::<f64>().unwrap_or(f64::NAN);
                if n < 10.0 {
                    "0-9"
                } else if n < 100.0 {
                    "10-99"
                } else if n < 1000.0 {
                    "100-999"
                } else if n < 10000.0 {
                    "1000-9999"
                } else {
                    "10000+"
                }
                .to_string()
            })
            .to_string();
    }
    out
}

pub fn looks_csv(input: &str) -> bool {
    csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .from_reader(input.as_bytes())
        .records()
        .collect::<Result<Vec<_>, _>>()
        .is_ok_and(|rows| rows.first().is_some_and(|r| r.len() > 1))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_digits_are_left_alone() {
        let opts = RestructureOptions {
            bucket_numerics: true,
            coarsen_dates: "month".into(),
            drop_columns: vec![],
        };
        let out = restructure("when\n２０２６-０１-０１\n", &opts).unwrap();
        assert!(out.contains("２０２６-０１-０１"));
        assert!(restructure("when\n2026-01-15\n", &opts).unwrap().contains("2026-01"));
    }
}
