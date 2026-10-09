use regex::bytes::{Regex, RegexBuilder};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entity {
    pub kind: String,
    pub value: String,
    pub start: usize,
    pub end: usize,
    pub risk: i32,
}

fn re(pattern: &str) -> Regex {
    RegexBuilder::new(pattern).unicode(false).build().unwrap()
}

pub fn detect(text: &str) -> Vec<Entity> {
    let patterns = [
        ("EMAIL", 9, r"(?i)\b[A-Z0-9._%+\-]+@[A-Z0-9.\-]+\.[A-Z]{2,}\b"),
        (
            "EMAIL",
            8,
            r"(?i)\b[A-Z0-9_%+\-]+(?:\s+dot\s+[A-Z0-9_%+\-]+)*\s+(?:at|\[at\]|\(at\))\s+[A-Z0-9\-]+(?:\s+dot\s+[A-Z0-9\-]+)+\b",
        ),
        (
            "EMAIL",
            8,
            r"(?i)\b[A-Z0-9._%+\-]+\s*(?:\[at\]|\(at\))\s*[A-Z0-9.\-]+\.[A-Z]{2,}\b",
        ),
        (
            "PHONE",
            7,
            r"\b(?:\+?1[\s.\-]?)?(?:\(?\d{3}\)?[\s.\-]?)\d{3}[\s.\-]?\d{4}\b",
        ),
        ("PHONE", 7, r"\+\d{1,3}(?:[\s().\-]*\d){7,12}\b"),
        ("PHONE", 7, r"\b0\d{2,4}[\s().\-]*\d{3,4}[\s().\-]*\d{3,4}\b"),
        ("SSN", 10, r"\b\d{3}-\d{2}-\d{4}\b"),
        ("ID_NUMBER", 9, r"(?i)\b[A-Z]{2}\s*\d{2}\s*\d{2}\s*\d{2}\s*[A-D]\b"),
        ("ID_NUMBER", 8, r"\b\d{3}[ -]\d{3}[ -]\d{2,3}\b"),
        (
            "IBAN_CANDIDATE",
            9,
            r"(?i)\b[A-Z]{2}\d{2}(?:\s?[A-Z0-9]{4}){2,7}(?:\s?[A-Z0-9]{1,4})?\b|\b[A-Z]{2}\d{2}[A-Z0-9]{11,30}\b",
        ),
        ("IP", 5, r"\b(?:\d{1,3}\.){3}\d{1,3}\b"),
        ("URL", 4, r#"\bhttps?://[^\s<>"']+"#),
        ("CARD_CANDIDATE", 10, r"\b(?:\d[ -]*?){13,19}\b"),
    ];
    let mut out = Vec::new();
    for (kind, risk, pattern) in patterns {
        for m in re(pattern).find_iter(text.as_bytes()) {
            let value = &text[m.start()..m.end()];
            let mut kind = kind;
            let mut risk = risk;
            if kind == "CARD_CANDIDATE" {
                if !valid_luhn(value) {
                    continue;
                }
                kind = "CARD";
            }
            if kind == "IBAN_CANDIDATE" {
                kind = "ACCOUNT";
                if !valid_iban(value) {
                    risk = 6;
                }
            }
            if kind == "IP" && !valid_ipv4(value) {
                continue;
            }
            out.push(Entity {
                kind: kind.into(),
                value: value.into(),
                start: m.start(),
                end: m.end(),
                risk,
            });
        }
    }
    out.extend(detect_likely_names(text));
    merge_entities(out)
}

pub fn risk_score(entities: &[Entity]) -> i32 {
    entities.iter().map(|e| e.risk).max().unwrap_or(0)
}

pub fn merge_entities(mut entities: Vec<Entity>) -> Vec<Entity> {
    entities.sort_by(|a, b| a.start.cmp(&b.start).then(b.end.cmp(&a.end)));
    let mut out = Vec::new();
    let mut i = 0;
    while i < entities.len() {
        let mut best = entities[i].clone();
        let mut cluster_end = best.end;
        i += 1;
        while i < entities.len() && entities[i].start < cluster_end {
            cluster_end = cluster_end.max(entities[i].end);
            if better(&entities[i], &best) {
                best = entities[i].clone();
            }
            i += 1;
        }
        out.push(best);
    }
    out.sort_by_key(|e| e.start);
    out
}

pub(crate) fn better(a: &Entity, b: &Entity) -> bool {
    if a.kind == "ACCOUNT" && (b.kind == "CARD" || b.kind == "PHONE") && a.start <= b.start && a.end >= b.end {
        return true;
    }
    if b.kind == "ACCOUNT" && (a.kind == "CARD" || a.kind == "PHONE") && b.start <= a.start && b.end >= a.end {
        return false;
    }
    a.risk > b.risk
        || (a.risk == b.risk
            && (a.end - a.start > b.end - b.start || (a.end - a.start == b.end - b.start && a.start < b.start)))
}

fn valid_luhn(s: &str) -> bool {
    let digits: Vec<u32> = s
        .bytes()
        .filter(|b| b.is_ascii_digit())
        .map(|b| (b - b'0') as u32)
        .collect();
    if digits.len() < 13 {
        return false;
    }
    let sum: u32 = digits
        .iter()
        .rev()
        .enumerate()
        .map(|(i, &d)| {
            if i % 2 == 0 {
                d
            } else {
                let n = d * 2;
                if n > 9 { n - 9 } else { n }
            }
        })
        .sum();
    sum.is_multiple_of(10)
}

fn valid_ipv4(s: &str) -> bool {
    let parts: Vec<_> = s.split('.').collect();
    parts.len() == 4
        && parts.iter().all(|p| {
            !p.is_empty()
                && p.len() <= 3
                && p.bytes().all(|b| b.is_ascii_digit())
                && p.parse::<u16>().is_ok_and(|n| n <= 255)
        })
}

fn valid_iban(value: &str) -> bool {
    let s = value.replace(' ', "").to_ascii_uppercase();
    if !(15..=34).contains(&s.len()) {
        return false;
    }
    let rearranged = format!("{}{}", &s[4..], &s[..4]);
    let mut rem = 0u32;
    for b in rearranged.bytes() {
        rem = match b {
            b'0'..=b'9' => (rem * 10 + (b - b'0') as u32) % 97,
            b'A'..=b'Z' => (rem * 100 + (b - b'A') as u32 + 10) % 97,
            _ => return false,
        };
    }
    rem == 1
}

const CUES: &[&str] = &[
    "ask", "call", "contact", "dear", "email", "hello", "hey", "hi", "message", "nickname", "alias", "ping", "tell",
    "text", "thanks", "thank", "cc", "attn",
];
const LEADING: &[&str] = &[
    "The",
    "This",
    "That",
    "These",
    "Those",
    "Please",
    "Our",
    "Your",
    "My",
    "His",
    "Her",
    "Their",
    "When",
    "Then",
    "Today",
    "Tomorrow",
    "Yesterday",
    "If",
    "And",
    "But",
    "Or",
    "So",
    "Also",
    "Note",
    "Mixed",
    "Fictional",
    "Mock",
    "Send",
    "Meet",
    "Callback",
    "Customer",
    "Patient",
    "User",
    "Owner",
    "Name",
    "From",
    "To",
    "Subject",
    "Re",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
    "January",
    "February",
    "March",
    "April",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
    "Die",
    "La",
    "El",
    "Le",
];
const NONPERSON: &[&str] = &[
    "Avenue",
    "Ave",
    "Boulevard",
    "Blvd",
    "City",
    "Cloud",
    "County",
    "Court",
    "Drive",
    "Inc",
    "Labs",
    "Lane",
    "Limited",
    "Ltd",
    "Mixlayer",
    "Platform",
    "Road",
    "Services",
    "Street",
    "Systems",
    "University",
    "Way",
    "York",
    "Analytics",
    "Board",
    "Center",
    "Committee",
    "Department",
    "Engineering",
    "Group",
    "Manager",
    "Office",
    "Report",
    "Request",
    "Review",
    "Square",
    "Support",
    "Team",
    "Project",
    "Program",
    "Plan",
    "Policy",
];
const PHRASES: &[&str] = &[
    "Amazon Web",
    "Google Cloud",
    "Los Angeles",
    "Mixlayer Labs",
    "New York",
    "San Francisco",
];

fn word_before(s: &str) -> &str {
    let end = s.trim_end_matches(|c: char| !c.is_alphabetic()).len();
    let start = s[..end]
        .char_indices()
        .rev()
        .take_while(|(_, c)| c.is_alphabetic())
        .last()
        .map_or(end, |(i, _)| i);
    &s[start..end]
}

fn person_cue_before(s: &str) -> bool {
    let w1 = word_before(s).to_lowercase();
    if CUES.contains(&w1.as_str())
        || [
            "say", "says", "signed", "named", "called", "by", "with", "dr", "mr", "mrs", "ms",
        ]
        .contains(&w1.as_str())
    {
        return true;
    }
    if w1 == "to" || w1 == "for" {
        let trimmed = s.trim_end_matches(|c: char| !c.is_alphabetic());
        let before = &trimmed[..trimmed.len() - word_before(trimmed).len()];
        return ["belongs", "belong", "talk", "spoke", "assigned", "sent"]
            .contains(&word_before(before).to_lowercase().as_str());
    }
    false
}

pub fn detect_likely_names(text: &str) -> Vec<Entity> {
    let pattern = re(r"\b[A-Z][a-z]+(?:['\-][A-Z]?[a-z]+)?(?:[ \t]*\n?[ \t]*[A-Z][a-z]+(?:['\-][A-Z]?[a-z]+)?){0,4}\b");
    let mut out = Vec::new();
    for m in pattern.find_iter(text.as_bytes()) {
        let mut toks: Vec<(usize, usize)> = Vec::new();
        let mut token_start = None;
        for (i, c) in text[m.start()..m.end()].char_indices() {
            if matches!(c, ' ' | '\t' | '\n' | '\r') {
                if let Some(start) = token_start.take() {
                    toks.push((m.start() + start, m.start() + i));
                }
            } else if token_start.is_none() {
                token_start = Some(i);
            }
        }
        if let Some(start) = token_start {
            toks.push((m.start() + start, m.end()));
        }
        let mut cue = false;
        while let Some((start, end)) = toks.first().copied() {
            let w = &text[start..end];
            if CUES.contains(&w.to_lowercase().as_str()) {
                cue = true;
            } else if !LEADING.contains(&w) {
                break;
            }
            toks.remove(0);
        }
        while let Some((start, end)) = toks.last().copied() {
            if !NONPERSON.contains(&&text[start..end]) {
                break;
            }
            toks.pop();
        }
        if toks.is_empty() {
            continue;
        }
        let start = toks[0].0;
        let end = toks.last().unwrap().1;
        let value = &text[start..end];
        if toks.len() == 1 && ((!cue && !person_cue_before(&text[..start])) || LEADING.contains(&value)) {
            continue;
        }
        if PHRASES.contains(&value)
            || value.split_whitespace().any(|p| NONPERSON.contains(&p))
            || ["at", "in", "near"].contains(&word_before(&text[..start]).to_lowercase().as_str())
        {
            continue;
        }
        out.push(Entity {
            kind: "PERSON".into(),
            value: value.into(),
            start,
            end,
            risk: 6,
        });
    }
    out
}
