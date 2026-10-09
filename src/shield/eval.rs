use std::collections::BTreeMap;

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

use super::{MemoryVault, detect_with, redact_entities};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvalCounts {
    pub gold: usize,
    pub found: usize,
    pub detected: usize,
    pub correct: usize,
    pub leaked: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EvalMetrics {
    pub recall: f64,
    pub precision: f64,
    pub leak_rate: f64,
    pub counts: EvalCounts,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EvalResult {
    pub detector: String,
    pub errors: usize,
    pub overall: EvalMetrics,
    pub by_kind: BTreeMap<String, EvalMetrics>,
}

#[derive(Deserialize)]
pub struct EvalGold {
    pub kind: String,
    pub text: String,
}

#[derive(Deserialize)]
pub struct EvalSample {
    pub id: String,
    pub text: String,
    pub entities: Vec<EvalGold>,
}

fn metrics(c: EvalCounts) -> EvalMetrics {
    EvalMetrics {
        recall: if c.gold > 0 {
            c.found as f64 / c.gold as f64
        } else {
            1.0
        },
        precision: if c.detected > 0 {
            c.correct as f64 / c.detected as f64
        } else {
            1.0
        },
        leak_rate: if c.gold > 0 {
            c.leaked as f64 / c.gold as f64
        } else {
            0.0
        },
        counts: c,
    }
}

pub fn read_eval_samples(input: &str) -> Result<Vec<EvalSample>> {
    let mut out = Vec::new();
    for (line, raw) in input.lines().enumerate() {
        let raw = raw.trim();
        if raw.is_empty() || raw.starts_with('#') {
            continue;
        }
        let sample: EvalSample = serde_json::from_str(raw).map_err(|e| anyhow!("sample line {}: {e}", line + 1))?;
        if sample.id.is_empty() {
            return Err(anyhow!("sample line {}: id is required", line + 1));
        }
        out.push(sample);
    }
    if out.is_empty() {
        return Err(anyhow!("sample contains no records"));
    }
    Ok(out)
}

pub fn evaluate_regex(samples: &[EvalSample]) -> Result<EvalResult> {
    let mut counts: BTreeMap<String, EvalCounts> = BTreeMap::new();
    let mut vault = MemoryVault::default();
    for sample in samples {
        let (entities, _) = detect_with(&sample.text, &[], None, false)?;
        let redacted = redact_entities(&mut vault, &sample.text, entities.clone(), false)?.text;
        let mut gold = Vec::new();
        for g in &sample.entities {
            if g.text.is_empty() {
                continue;
            }
            for (start, _) in sample.text.match_indices(&g.text) {
                gold.push((g.kind.as_str(), start, start + g.text.len()));
            }
        }
        for &(kind, start, end) in &gold {
            let c = counts.entry(kind.into()).or_default();
            c.gold += 1;
            if entities.iter().any(|e| e.start <= start && e.end >= end) {
                c.found += 1;
            }
            if redacted.contains(&sample.text[start..end]) {
                c.leaked += 1;
            }
        }
        for e in &entities {
            let c = counts.entry(e.kind.clone()).or_default();
            c.detected += 1;
            if gold.iter().any(|&(_, start, end)| e.start < end && start < e.end) {
                c.correct += 1;
            }
        }
    }
    let mut all = EvalCounts::default();
    let by_kind = counts
        .into_iter()
        .map(|(kind, c)| {
            all.gold += c.gold;
            all.found += c.found;
            all.detected += c.detected;
            all.correct += c.correct;
            all.leaked += c.leaked;
            (kind, metrics(c))
        })
        .collect();
    Ok(EvalResult {
        detector: "regex".into(),
        errors: 0,
        overall: metrics(all),
        by_kind,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthetic_regex_parity() {
        compare(
            include_str!("../../testdata/shield-eval/synthetic.jsonl"),
            include_str!("../../parity/fixtures/go-eval-synthetic-regex.json"),
        );
    }
    #[test]
    fn adversarial_regex_parity() {
        compare(
            include_str!("../../testdata/shield-eval/adversarial.jsonl"),
            include_str!("../../parity/fixtures/go-eval-adversarial-regex.json"),
        );
    }
    fn compare(input: &str, expected: &str) {
        let samples = read_eval_samples(input).unwrap();
        let actual = evaluate_regex(&samples).unwrap();
        let want: serde_json::Value = serde_json::from_str(expected).unwrap();
        let want = &want["results"][0];
        assert_eq!(actual.detector, want["detector"]);
        assert_eq!(actual.errors, want["errors"]);
        assert_metrics(&actual.overall, &want["overall"]);
        assert_eq!(actual.by_kind.len(), want["by_kind"].as_object().unwrap().len());
        for (kind, metrics) in &actual.by_kind {
            assert_metrics(metrics, &want["by_kind"][kind]);
        }
    }
    fn assert_metrics(actual: &EvalMetrics, want: &serde_json::Value) {
        assert_eq!(serde_json::to_value(&actual.counts).unwrap(), want["counts"]);
        for (key, got) in [
            ("recall", actual.recall),
            ("precision", actual.precision),
            ("leak_rate", actual.leak_rate),
        ] {
            let expected = want[key].as_f64().unwrap();
            assert!((got - expected).abs() < 1e-15, "{key}: {got} != {expected}");
        }
    }
}
