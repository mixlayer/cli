use anyhow::Result;

use super::{Entity, Vault, detect, risk_score};

#[derive(Clone, Debug)]
pub struct MaskedEntity {
    pub kind: String,
    pub value: String,
    pub token: String,
    pub start: usize,
    pub end: usize,
    pub risk: i32,
}

#[derive(Clone, Debug)]
pub struct RedactionResult {
    pub text: String,
    pub entities: Vec<MaskedEntity>,
    pub risk: i32,
}

pub fn redact(vault: &mut impl Vault, text: &str, reveal: bool) -> Result<RedactionResult> {
    redact_entities(vault, text, detect(text), reveal)
}

pub fn redact_entities(
    vault: &mut impl Vault,
    text: &str,
    mut entities: Vec<Entity>,
    reveal: bool,
) -> Result<RedactionResult> {
    let mut masked = Vec::new();
    for e in &mut entities {
        let entry = vault.ensure_entry(&e.kind, &e.value)?;
        masked.push(MaskedEntity {
            kind: e.kind.clone(),
            value: if reveal { e.value.clone() } else { String::new() },
            token: entry.token.clone(),
            start: e.start,
            end: e.end,
            risk: e.risk,
        });
        e.value = entry.token;
    }
    let risk = risk_score(&entities);
    entities.sort_by_key(|e| std::cmp::Reverse(e.start));
    let mut out = text.to_string();
    for e in entities {
        out.replace_range(e.start..e.end, &e.value);
    }
    Ok(RedactionResult {
        text: out,
        entities: masked,
        risk,
    })
}

pub fn rehydrate(vault: &impl Vault, text: &str) -> Result<String> {
    let mapping = vault.token_map()?;
    let mut tokens: Vec<_> = mapping.keys().collect();
    tokens.sort_by_key(|t| std::cmp::Reverse(t.len()));
    let mut out = text.to_string();
    for token in tokens {
        out = out.replace(token, &mapping[token]);
    }
    Ok(out)
}
