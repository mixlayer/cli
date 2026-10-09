// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

pub mod agent_context;
pub mod agent_setup;
pub mod api;
pub mod cli;
pub mod exit;
pub mod help_text;
pub mod ledger;
pub mod mcp;
pub mod output;
pub mod paths;
pub mod profiles;
pub mod shield;
pub mod shield_cmd;
pub mod skill;
pub mod spec;
pub mod spec_types;
pub mod store;
pub mod which;

/// Env vars set by coding agents. When one is present, agent defaults apply
/// unless `MIXLAYER_NO_AGENT_DETECT=1`.
pub const AGENT_ENV: &[&str] = &[
    "CLAUDECODE",
    "CLAUDE_CODE",
    "CODEX_SANDBOX",
    "CURSOR_AGENT",
    "GEMINI_CLI",
    "AI_AGENT",
];

pub fn agent_detected() -> bool {
    if std::env::var("MIXLAYER_NO_AGENT_DETECT").as_deref() == Ok("1") {
        return false;
    }
    AGENT_ENV
        .iter()
        .any(|v| std::env::var_os(v).is_some_and(|x| !x.is_empty()))
}

/// Global flags after agent defaults are applied.
#[derive(Clone, Debug, Default)]
pub struct Globals {
    pub agent: bool,
    pub json: bool,
    pub compact: bool,
    pub no_input: bool,
    pub no_color: bool,
    pub yes: bool,
    pub dry_run: bool,
    pub select: Option<String>,
}
