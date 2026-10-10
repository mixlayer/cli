use super::commands::ResultRow;
use crate::Globals;
use std::io::IsTerminal;

pub fn want(g: &Globals) -> bool {
    !g.json && !g.agent && !g.csv && !g.plain && !g.quiet && std::io::stdout().is_terminal()
}
fn color(g: &Globals) -> bool {
    want(g) && !g.no_color && std::env::var_os("NO_COLOR").is_none()
}
fn paint(s: &str, code: u8, g: &Globals) -> String {
    if color(g) {
        format!("\x1b[{code}m{s}\x1b[0m")
    } else {
        s.into()
    }
}
fn bold(s: &str, g: &Globals) -> String {
    paint(s, 1, g)
}
fn dim(s: &str, g: &Globals) -> String {
    paint(s, 2, g)
}
fn green(s: &str, g: &Globals) -> String {
    paint(s, 32, g)
}
fn yellow(s: &str, g: &Globals) -> String {
    paint(s, 33, g)
}
fn magenta(s: &str, g: &Globals) -> String {
    paint(s, 35, g)
}
fn red(s: &str, g: &Globals) -> String {
    paint(s, 31, g)
}

pub fn round(v: f64) -> f64 {
    (v * 1e12).round() / 1e12
}
pub fn usd(v: f64) -> String {
    let v = round(v);
    if v == 0.0 {
        return "$0".into();
    }
    let s = format!("{v:.12}");
    format!("${}", s.trim_end_matches('0').trim_end_matches('.'))
}
pub fn strip_confidence_tag(s: &str) -> String {
    let t = s.trim_end_matches([' ', '\n', '\t', '*']);
    if let Some(i) = t.to_ascii_uppercase().rfind("CONFIDENCE:")
        && !t[i..].contains('\n')
        && t.len() - i <= 24
    {
        return t[..i].trim_end_matches([' ', '\t', '*', '\n']).into();
    }
    s.into()
}
fn block_text(r: &ResultRow, note: &str, g: &Globals) -> String {
    let mut meta = Vec::new();
    if r.latency_ms > 0 {
        meta.push(format!("{:.2}s", r.latency_ms as f64 / 1000.0));
    }
    if r.total_tokens > 0 {
        meta.push(format!("{} tokens", r.total_tokens));
    }
    meta.push(if r.cost_unknown {
        "cost unknown".into()
    } else {
        usd(r.cost_usd)
    });
    let mut header = format!("{}  {}", bold(&r.model, g), dim(&meta.join(" · "), g));
    if !note.is_empty() {
        header.push_str(&format!("  {note}"));
    }
    let mut out = format!("{header}\n");
    if let Some(error) = &r.error {
        out.push_str(&format!("  {}\n", red(&format!("error: {error}"), g)));
    } else {
        let answer = strip_confidence_tag(&r.answer);
        if answer.trim().is_empty() {
            out.push_str(&format!("  {}\n", dim("(empty answer)", g)));
        } else {
            for line in answer.trim().lines() {
                out.push_str(&format!("  {line}\n"));
            }
        }
    }
    out.push('\n');
    out
}
fn block(r: &ResultRow, note: &str, g: &Globals) {
    print!("{}", block_text(r, note, g));
}
fn sum(results: &[ResultRow]) -> Option<f64> {
    if results.iter().any(|r| r.cost_unknown) {
        None
    } else {
        Some(results.iter().map(|r| r.cost_usd).sum())
    }
}
pub fn ladder(results: &[ResultRow], first: &str, g: &Globals) {
    for r in results {
        let note = if r.model == first {
            green("first confident answer", g)
        } else {
            String::new()
        };
        block(r, &note, g);
    }
    if let Some(cost) = sum(results) {
        println!("{} across {} models", bold(&usd(cost), g), results.len());
    }
}
pub fn escalate(
    results: &[ResultRow],
    cost: Option<f64>,
    baseline: Option<f64>,
    saved: Option<f64>,
    reason: &str,
    g: &Globals,
) {
    print!("{}", escalate_text(results, cost, baseline, saved, reason, g));
}
fn escalate_text(
    results: &[ResultRow],
    cost: Option<f64>,
    baseline: Option<f64>,
    saved: Option<f64>,
    reason: &str,
    g: &Globals,
) -> String {
    let mut out = String::new();
    for (i, r) in results.iter().enumerate() {
        let note = r
            .checker_score
            .map(|score| {
                let s = if i == results.len() - 1 && r.error.is_none() {
                    format!("confidence {score:.2}, stopped here")
                } else {
                    format!("confidence {score:.2}, escalating")
                };
                if i == results.len() - 1 && r.error.is_none() {
                    green(&s, g)
                } else {
                    yellow(&s, g)
                }
            })
            .unwrap_or_default();
        out.push_str(&block_text(r, &note, g));
    }
    if let Some(c) = cost {
        let mut line = format!("Spent {}", bold(&usd(c), g));
        if let Some(b) = baseline {
            line.push_str(&format!(" · the top model would have cost {}", usd(b)));
            if let Some(s) = saved.filter(|s| *s > 0.0) {
                line.push_str(&format!(" · {}", green(&format!("saved {}", usd(s)), g)));
            }
        } else if !reason.is_empty() {
            line.push_str(&dim(&format!(" ({reason})"), g));
        }
        out.push_str(&format!("{line}\n"));
    }
    out
}
#[allow(dead_code)] // Shared Go formatter retained for the shield command wiring.
pub fn format_masked_counts(counts: &std::collections::HashMap<String, i64>) -> (i64, String) {
    let mut entries: Vec<_> = counts.iter().filter(|(_, n)| **n > 0).collect();
    entries.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    let total = entries.iter().map(|(_, n)| **n).sum();
    (
        total,
        entries
            .iter()
            .map(|(kind, n)| format!("{n} {kind}"))
            .collect::<Vec<_>>()
            .join(", "),
    )
}
pub fn council(members: &[ResultRow], judge: &[ResultRow], g: &Globals) {
    for r in members {
        block(r, "", g);
    }
    for r in judge {
        println!("{}", magenta("Judge", g));
        block(r, "", g);
    }
    if let Some(cost) = sum(&members.iter().chain(judge).cloned().collect::<Vec<_>>()) {
        println!("{} total", bold(&usd(cost), g));
    }
}
pub fn compare(cheap: &ResultRow, frontier: &ResultRow, delta: Option<f64>, note: &str, g: &Globals) {
    block(cheap, &dim("cheap", g), g);
    block(frontier, &dim("frontier", g), g);
    if let Some(d) = delta {
        println!("The frontier answer cost {} more.", bold(&usd(d), g));
    }
    if !note.is_empty() {
        println!("{}", dim(note, g));
    }
}
pub fn savings(runs: usize, unknown: usize, tokens: i64, actual: f64, baseline: f64, vs: &str, g: &Globals) {
    println!("{}  {runs} runs, {tokens} tokens", bold("Your usage", g));
    let label = format!("at {vs} prices");
    println!("  {:width$}  {}", "you paid", usd(actual), width = label.len());
    println!("  {:width$}  {}", label, usd(baseline), width = label.len());
    println!(
        "  {:width$}  {}",
        "saved",
        green(&usd(baseline - actual), g),
        width = label.len()
    );
    if unknown > 0 {
        println!(
            "{}",
            dim(&format!("  {unknown} runs with unknown cost were left out"), g)
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn format_usd_matches_go() {
        for (v, s) in [
            (0.0, "$0"),
            (0.0000042000000000000004, "$0.0000042"),
            (0.00505, "$0.00505"),
            (1.25, "$1.25"),
        ] {
            assert_eq!(usd(v), s);
        }
    }
    #[test]
    fn confidence_tag_matches_go() {
        for (v, s) in [
            ("No, 91 = 7*13. CONFIDENCE: high", "No, 91 = 7*13."),
            ("Yes.\n**CONFIDENCE: low**", "Yes."),
            ("391", "391"),
            ("CONFIDENCE: is a word I like\nok", "CONFIDENCE: is a word I like\nok"),
        ] {
            assert_eq!(strip_confidence_tag(v), s);
        }
    }
    #[test]
    fn escalate_and_masked_counts_match_go() {
        let g = Globals {
            no_color: true,
            ..Default::default()
        };
        let rows = [
            ResultRow {
                model: "cheap/a".into(),
                answer: "maybe".into(),
                total_tokens: 10,
                latency_ms: 400,
                checker_score: Some(0.4),
                ..Default::default()
            },
            ResultRow {
                model: "mid/b".into(),
                answer: "391".into(),
                total_tokens: 12,
                latency_ms: 650,
                cost_usd: 0.0000039,
                checker_score: Some(0.9),
                ..Default::default()
            },
        ];
        let out = escalate_text(&rows, Some(0.0000039), Some(0.00505), Some(0.0050461), "", &g);
        for phrase in [
            "cheap/a",
            "confidence 0.40, escalating",
            "mid/b",
            "confidence 0.90, stopped here",
            "  391",
            "Spent $0.0000039",
            "would have cost $0.00505",
            "saved $0.0050461",
        ] {
            assert!(out.contains(phrase), "{phrase}: {out}");
        }
        let counts = std::collections::HashMap::from([("EMAIL".into(), 1), ("PERSON".into(), 2), ("PHONE".into(), 0)]);
        assert_eq!(format_masked_counts(&counts), (3, "2 PERSON, 1 EMAIL".into()));
    }
}
