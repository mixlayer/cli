use anyhow::{Result, anyhow};
use regex::RegexBuilder;
use serde::Deserialize;

use super::{Entity, better, detect, merge_entities};

pub type ModelCall<'a> = dyn Fn(&str) -> Result<String> + 'a;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DictionaryTerm {
    pub term: String,
    pub kind: String,
}

pub fn kind_risk(kind: &str) -> i32 {
    match kind {
        "PERSON" | "ADDRESS" | "DATE_OF_BIRTH" => 8,
        "ORG" | "URL" => 4,
        "LOCATION" | "OTHER" | "IP" => 5,
        "ID_NUMBER" | "SSN" | "CARD" | "CARD_CANDIDATE" => 10,
        "MEDICAL" | "ACCOUNT" | "EMAIL" | "IBAN_CANDIDATE" => 9,
        "PHONE" => 7,
        _ => 5,
    }
}

pub fn valid_kind(kind: &str) -> bool {
    matches!(
        kind,
        "PERSON"
            | "ADDRESS"
            | "ORG"
            | "LOCATION"
            | "DATE_OF_BIRTH"
            | "ID_NUMBER"
            | "MEDICAL"
            | "ACCOUNT"
            | "OTHER"
            | "EMAIL"
            | "PHONE"
            | "SSN"
            | "CARD"
            | "IP"
            | "URL"
    )
}

pub fn dictionary_matches(text: &str, terms: &[DictionaryTerm]) -> Vec<Entity> {
    let mut out = Vec::new();
    for term in terms {
        if term.term.is_empty() {
            continue;
        }
        let re = RegexBuilder::new(&regex::escape(&term.term))
            .case_insensitive(true)
            .build()
            .unwrap();
        for m in re.find_iter(text) {
            let before = text[..m.start()].chars().next_back();
            let after = text[m.end()..].chars().next();
            let word = |c: char| c.is_alphanumeric() || c == '_';
            if before.is_some_and(word) || after.is_some_and(word) {
                continue;
            }
            out.push(Entity {
                kind: term.kind.clone(),
                value: m.as_str().into(),
                start: m.start(),
                end: m.end(),
                risk: kind_risk(&term.kind),
            });
        }
    }
    out.sort_by_key(|e| e.start);
    let mut merged: Vec<Entity> = Vec::new();
    for e in out {
        if let Some(last) = merged.last_mut().filter(|last| e.start < last.end) {
            last.end = last.end.max(e.end);
            last.value = text[last.start..last.end].into();
            last.risk = last.risk.max(e.risk);
        } else {
            merged.push(e);
        }
    }
    merged
}

pub fn chunks(text: &str) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut start = 0;
    while start < chars.len() {
        let mut end = start + 6000;
        if end >= chars.len() {
            out.push(chars[start..].iter().collect());
            break;
        }
        for i in (start + 3001..end).rev() {
            if matches!(chars[i], '\n' | '.' | '!' | '?') {
                end = i + 1;
                break;
            }
        }
        out.push(chars[start..end].iter().collect());
        start = end - 200;
    }
    out
}

#[derive(Deserialize)]
struct Response {
    entities: Option<Vec<ModelEntity>>,
}
#[derive(Deserialize)]
struct ModelEntity {
    #[serde(default)]
    kind: String,
    #[serde(default)]
    text: String,
}

fn go_json_string(text: &str) -> Result<String> {
    Ok(serde_json::to_string(text)?
        .replace('&', "\\u0026")
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029"))
}

fn cover_overlaps(text: &str, mut entities: Vec<Entity>) -> Vec<Entity> {
    if entities.is_empty() {
        return entities;
    }
    entities.sort_by(|a, b| a.start.cmp(&b.start).then(b.end.cmp(&a.end)));
    let mut out = Vec::new();
    let mut best = entities[0].clone();
    let (mut start, mut end) = (best.start, best.end);
    for e in entities.into_iter().skip(1) {
        if e.start < end {
            end = end.max(e.end);
            if better(&e, &best) {
                best = e;
            }
        } else {
            best.start = start;
            best.end = end;
            best.value = text[start..end].into();
            out.push(best);
            start = e.start;
            end = e.end;
            best = e;
        }
    }
    best.start = start;
    best.end = end;
    best.value = text[start..end].into();
    out.push(best);
    out
}

pub fn detect_with(
    text: &str,
    dictionary: &[DictionaryTerm],
    call: Option<&ModelCall<'_>>,
    fallback: bool,
) -> Result<(Vec<Entity>, bool)> {
    let (entities, used, _) = detect_with_stats(text, dictionary, call, fallback)?;
    Ok((entities, used))
}

pub fn detect_with_stats(
    text: &str,
    dictionary: &[DictionaryTerm],
    call: Option<&ModelCall<'_>>,
    fallback: bool,
) -> Result<(Vec<Entity>, bool, usize)> {
    let regex = detect(text);
    let mut entities = regex.clone();
    let mut ignored = 0;
    let mut used_fallback = false;
    if let Some(call) = call {
        for chunk in chunks(text) {
            let prompt = format!(
                "Return ONLY strict JSON of this shape: {{\"entities\":[{{\"kind\":\"PERSON|ADDRESS|ORG|LOCATION|DATE_OF_BIRTH|ID_NUMBER|MEDICAL|ACCOUNT|OTHER\",\"text\":\"<exact substring>\"}}]}}. Identify sensitive spans. Copy each text verbatim. Do not redact, paraphrase, or invent. Text: {}",
                go_json_string(&chunk)?
            );
            let result = call(&prompt).and_then(|answer| {
                serde_json::from_str::<Response>(&answer)
                    .map_err(Into::into)
                    .and_then(|r| {
                        r.entities
                            .ok_or_else(|| anyhow!("model detector returned invalid JSON entities"))
                    })
            });
            match result {
                Ok(items) => {
                    for item in items {
                        if item.text.trim().is_empty() {
                            ignored += 1;
                            continue;
                        }
                        let mut kind = item.kind.trim().to_uppercase();
                        if !valid_kind(&kind) {
                            kind = "OTHER".into();
                        }
                        let mut matched = false;
                        for (start, _) in text.match_indices(&item.text) {
                            matched = true;
                            entities.push(Entity {
                                kind: kind.clone(),
                                value: item.text.clone(),
                                start,
                                end: start + item.text.len(),
                                risk: kind_risk(&kind),
                            });
                        }
                        if !matched {
                            ignored += 1;
                        }
                    }
                }
                Err(e) => {
                    if !fallback {
                        return Err(anyhow!("model detector failed: {e}"));
                    }
                    entities = regex.clone();
                    used_fallback = true;
                    break;
                }
            }
        }
    }
    entities = cover_overlaps(text, entities);
    for mut d in dictionary_matches(text, dictionary) {
        let mut i = 0;
        while i < entities.len() {
            let e = &entities[i];
            if e.start < d.end && d.start < e.end {
                d.start = d.start.min(e.start);
                d.end = d.end.max(e.end);
                d.value = text[d.start..d.end].into();
                d.risk = d.risk.max(e.risk);
                entities.remove(i);
                i = 0;
            } else {
                i += 1;
            }
        }
        entities.push(d);
    }
    Ok((merge_entities(entities), used_fallback, ignored))
}
