use std::collections::{HashMap, HashSet};

use anyhow::Result;
use regex::bytes::Regex;

use super::Vault;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct RestoreReport {
    pub text: String,
    pub restored_count: usize,
    pub unrestored_count: usize,
    pub unrestored_tokens: Vec<String>,
    pub repairs: Vec<String>,
}

pub fn rehydrate_checked(vault: &impl Vault, answer: &str) -> Result<RestoreReport> {
    restore(vault, answer, None)
}

pub fn rehydrate_checked_for_tokens(vault: &impl Vault, answer: &str, allowed: &[&str]) -> Result<RestoreReport> {
    restore(vault, answer, Some(allowed.iter().copied().collect()))
}

fn token_suffix(token: &str) -> Option<String> {
    let suffix = token
        .rsplit_once('_')?
        .1
        .chars()
        .filter(|c| !matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0c'))
        .collect::<String>()
        .to_ascii_lowercase();
    (suffix.len() >= 8 && suffix.bytes().all(|b| b.is_ascii_hexdigit())).then_some(suffix)
}

fn restore(vault: &impl Vault, answer: &str, allow: Option<HashSet<&str>>) -> Result<RestoreReport> {
    let all = vault.token_map()?;
    let mut mapping = HashMap::new();
    let mut folded: HashMap<String, Vec<String>> = HashMap::new();
    let mut by_suffix: HashMap<String, Vec<String>> = HashMap::new();
    for (token, value) in all {
        folded.entry(token.to_uppercase()).or_default().push(token.clone());
        if allow.as_ref().is_none_or(|set| set.contains(token.as_str())) {
            if allow.is_some()
                && let Some(sfx) = token_suffix(&token)
            {
                by_suffix.entry(sfx).or_default().push(token.clone());
            }
            mapping.insert(token, value);
        }
    }
    let placeholder = Regex::new(r"\b[A-Za-z]+(?:_[A-Za-z]+)*\s*_\s*[0-9a-fA-F](?:\s*[0-9a-fA-F]){7,15}\b")?;
    let mut report = RestoreReport::default();
    let mut seen = HashSet::new();
    let mut prev = 0;
    for m in placeholder.find_iter(answer.as_bytes()) {
        let candidate = &answer[m.start()..m.end()];
        report.text.push_str(&answer[prev..m.start()]);
        if let Some(value) = mapping.get(candidate) {
            report.text.push_str(value);
            report.restored_count += 1;
        } else {
            let normalized = candidate
                .chars()
                .filter(|c| !matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0c'))
                .collect::<String>()
                .to_uppercase();
            let matches = folded.get(&normalized);
            let exact = matches
                .filter(|v| v.len() == 1)
                .and_then(|v| mapping.get(&v[0]).map(|value| (value, &v[0])));
            let repaired = exact.or_else(|| {
                if matches.is_some() {
                    return None;
                }
                let suffix = token_suffix(&normalized)?;
                let same = by_suffix.get(&suffix).filter(|v| v.len() == 1)?;
                mapping.get(&same[0]).map(|value| (value, &same[0]))
            });
            if let Some((value, token)) = repaired {
                report.text.push_str(value);
                report.restored_count += 1;
                report.repairs.push(format!("{candidate} -> {token}"));
            } else {
                report.text.push_str(candidate);
                report.unrestored_count += 1;
                if seen.insert(candidate) {
                    report.unrestored_tokens.push(candidate.into());
                }
            }
        }
        prev = m.end();
    }
    report.text.push_str(&answer[prev..]);
    Ok(report)
}
