// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

//! `which`: maps a plain-language capability ("compare models") to the
//! command that does it. Same index and scoring as the Go CLI.

use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct Entry {
    pub command: &'static str,
    pub description: &'static str,
    #[serde(skip_serializing_if = "str::is_empty")]
    pub group: &'static str,
    #[serde(skip)]
    pub experimental: bool,
}

const fn e(command: &'static str, description: &'static str) -> Entry {
    Entry {
        command,
        description,
        group: "",
        experimental: false,
    }
}

pub static INDEX: &[Entry] = &[
    Entry {
        command: "decide",
        description: "Evaluate typed Jini classification decisions: noul, choice or score; gate CI with fail-if conditions.",
        group: "classification decisions jini",
        experimental: true,
    },
    Entry {
        command: "decide batch",
        description: "Classify CSV or JSONL rows with shared decision questions and bounded concurrency.",
        group: "classification csv jsonl batch",
        experimental: true,
    },
    e(
        "doctor",
        "Check API connectivity and verify authentication credentials.",
    ),
    e("auth status", "Check whether a Mixlayer API key is configured."),
    e(
        "keys create",
        "Create an organization API key with chosen permissions (inference, api-read, api-write, api-admin) through the Platform API; the secret is shown once.",
    ),
    e(
        "keys list",
        "List the organization's API keys with their permissions and expiry, never their secrets.",
    ),
    e(
        "keys rotate",
        "Rotate an API key's secret; anything using the old secret stops working.",
    ),
    e(
        "agent setup",
        "Install the Mixlayer skill and MCP server into Claude Code, Codex or Cursor; --platform adds the hosted Platform MCP server for organizations, members, API keys, analytics and billing.",
    ),
    e(
        "shield eval",
        "Evaluate detector recall, precision, and leaks on labeled JSONL samples.",
    ),
    e(
        "shield dict",
        "Manage local dictionary terms used by every shield detector.",
    ),
    e(
        "shield ingest",
        "Split large CSV/text inputs into deterministic tranches, redact each tranche through one shared local vault, and reassemble a masked corpus with a manifest.",
    ),
    e(
        "shield ask",
        "Redact local data, run an outbound PII tripwire, send only the masked payload to a frontier model, rehydrate the answer locally, and save a privacy receipt.",
    ),
    e(
        "shield scan",
        "Detect high-risk PII locally with CI-friendly exit codes and no upstream model call.",
    ),
    e(
        "shield redact",
        "Write a masked copy of a file and populate the local token-to-value vault with stable pseudonyms.",
    ),
    e(
        "shield restructure",
        "Drop columns, bucket numeric values, and coarsen dates to reduce re-identification risk before prompting.",
    ),
    e(
        "vault",
        "List, rehydrate, rotate, or purge the local reversible pseudonym vault without sending values upstream.",
    ),
    e(
        "shield audit",
        "Inspect outbound payload hashes, byte counts, model IDs, masked entity counts, and residual leak counts for shielded sends.",
    ),
    e(
        "ladder",
        "Run one prompt across selected Mixlayer model rungs, optionally comparing reasoning_content, latency, token usage, and cost.",
    ),
    e(
        "escalate",
        "Start on a cheap rung and climb only when the prior answer appears insufficiently confident.",
    ),
    e(
        "council",
        "Fan out to member model rungs and ask a judge model to synthesize answers and flag disagreements.",
    ),
    e(
        "ask",
        "Ask Mixlayer, optionally request reasoning_content, and save prompt, answer, model, seed, token usage, cost, and raw response locally.",
    ),
    e(
        "replay",
        "Re-run a saved prompt with its original model and seed to check determinism or drift.",
    ),
    e(
        "grep",
        "Search saved prompts, answers, and reasoning traces through the local FTS ledger.",
    ),
    e(
        "models query",
        "Query a local model cache with DSL terms like ctx>=128k tools reasoning, seed it with documented and console-visible model IDs, and optionally refresh it from live /models.",
    ),
    e(
        "sql",
        "Run SELECT-only SQLite queries over the local ledger, vault, audit, ladder, and model cache tables.",
    ),
    e(
        "savings",
        "Roll up saved ledger cost against static GPT or Claude frontier baselines.",
    ),
    e(
        "compare",
        "Run a prompt through a cheap rung and frontier rung, report the cost delta, and attach a local-history quality note.",
    ),
];

#[derive(Clone, Debug, Serialize)]
pub struct Match {
    pub entry: Entry,
    pub score: i32,
}

pub fn active_index(jini: bool) -> Vec<Entry> {
    INDEX.iter().filter(|e| jini || !e.experimental).cloned().collect()
}

/// Up to `limit` best matches, highest score first, ties in index order.
/// An empty query lists every entry at score 0.
pub fn rank(index: &[Entry], query: &str, limit: usize) -> Vec<Match> {
    let limit = if limit == 0 { 3 } else { limit };
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return index
            .iter()
            .map(|e| Match {
                entry: e.clone(),
                score: 0,
            })
            .collect();
    }
    let tokens: Vec<&str> = q.split_whitespace().collect();
    let mut scored: Vec<Match> = index
        .iter()
        .map(|e| Match {
            entry: e.clone(),
            score: score(e, &q, &tokens),
        })
        .collect();
    scored.sort_by_key(|m| std::cmp::Reverse(m.score)); // stable: ties keep index order
    scored.retain(|m| m.score > 0);
    scored.truncate(limit);
    scored
}

fn score(e: &Entry, q: &str, tokens: &[&str]) -> i32 {
    let mut s = 0;
    let cmd = e.command.to_lowercase();
    let desc = e.description.to_lowercase();
    let group = e.group.to_lowercase();
    if q.contains("compare") && (q.contains("model") || q.contains("rung")) {
        match e.command {
            "compare" => s += 8,
            "ladder" => s += 6,
            _ => {}
        }
    }
    if q.contains("auth") && (q.contains("check") || q.contains("verify")) && e.command == "doctor" {
        s += 8;
    }
    let cmd_tokens: Vec<&str> = cmd.split_whitespace().collect();
    for qt in tokens {
        if cmd_tokens.contains(qt) {
            s += 3;
        }
    }
    if cmd.contains(q) {
        s += 2;
    }
    if desc.contains(q) {
        s += 2;
    }
    if !group.is_empty() && tokens.iter().any(|qt| group.contains(qt)) {
        s += 1;
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compare_models_prefers_compare_then_ladder() {
        let m = rank(&active_index(false), "compare models", 3);
        assert_eq!(m[0].entry.command, "compare");
        assert_eq!(m[1].entry.command, "ladder");
    }

    #[test]
    fn no_match_is_empty_and_empty_lists_all() {
        assert!(rank(&active_index(false), "zzzz", 3).is_empty());
        assert_eq!(rank(&active_index(false), "", 3).len(), INDEX.len() - 2);
        assert_eq!(active_index(true).len(), INDEX.len());
    }
}
