// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

//! Builds the clap command tree from the table in `spec.rs`, and the global
//! flags every command accepts.

use clap::{Arg, ArgAction, Command};

use crate::spec_types::{Cmd, Kind};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub const ABOUT: &str = "Use Mixlayer models from your terminal and your coding agent";

pub const LONG_ABOUT: &str = "Use Mixlayer models from your terminal and your coding agent.

Start here:
  mxl doctor                      check your key and connection
  mxl models list --live-prices   see what you can call and what it costs
  mxl ask \"question\" --model <model-id>

Compare and save:   ladder, escalate, council, compare, savings
Private data:       shield ask, shield scan, shield redact, vault
Local ledger:       grep, replay, sql
Agents:             --agent, agent setup, agent-context, which

Add --agent to any command for JSON output and no prompts.";

/// (name, kind, usage, default). Same names and meanings as the Go CLI.
pub const GLOBAL_FLAGS: &[(&str, Kind, &str, Option<&str>)] = &[
    (
        "agent",
        Kind::Bool,
        "Set all agent-friendly defaults (--json --compact --no-input --no-color --yes)",
        None,
    ),
    (
        "compact",
        Kind::Bool,
        "Return only key fields (id, name, status, timestamps) for minimal token usage",
        None,
    ),
    ("config", Kind::Str, "Config file path", None),
    ("csv", Kind::Bool, "Output as CSV (table and array responses)", None),
    ("dry-run", Kind::Bool, "Show request without sending", None),
    (
        "home",
        Kind::Str,
        "Root directory for config, data, state, and cache files",
        None,
    ),
    (
        "human-friendly",
        Kind::Bool,
        "Enable colored output and rich formatting",
        None,
    ),
    ("json", Kind::Bool, "Output as JSON", None),
    ("no-cache", Kind::Bool, "Bypass response cache", None),
    ("no-color", Kind::Bool, "Disable colored output", None),
    (
        "no-input",
        Kind::Bool,
        "Disable all interactive prompts (for CI/agents)",
        None,
    ),
    ("plain", Kind::Bool, "Output as plain tab-separated text", None),
    (
        "profile",
        Kind::Str,
        "Apply values from a saved profile (see 'mxl profile list')",
        None,
    ),
    ("quiet", Kind::Bool, "Bare output, one value per line", None),
    (
        "rate-limit",
        Kind::Float,
        "Max requests per second (0 to disable)",
        Some("0"),
    ),
    (
        "select",
        Kind::Str,
        "Comma-separated fields to include in output (e.g. --select id,name,status)",
        None,
    ),
    ("timeout", Kind::Duration, "Request timeout", Some("1m0s")),
    (
        "yes",
        Kind::Bool,
        "Skip confirmation prompts (for agents and scripts)",
        None,
    ),
];

/// Id of the hidden catch-all for extra positional words.
pub const EXTRA_ARGS: &str = "__extra";

/// All positional words for a command, in order (declared, then extra).
pub fn positional_words(c: &Cmd, m: &clap::ArgMatches) -> Vec<String> {
    let mut out = Vec::new();
    for p in c.positionals() {
        if let Ok(Some(v)) = m.try_get_many::<String>(&p.name) {
            out.extend(v.cloned());
        }
    }
    if let Ok(Some(v)) = m.try_get_many::<String>(EXTRA_ARGS) {
        out.extend(v.cloned());
    }
    out
}

pub fn jini_enabled() -> bool {
    std::env::var("MIXLAYER_ENABLE_JINI").as_deref() == Ok("1")
}

/// The full clap tree.
pub fn build() -> Command {
    let mut root = Command::new("mxl")
        .version(VERSION)
        .about(ABOUT)
        .long_about(LONG_ABOUT)
        .subcommand_required(false)
        .arg_required_else_help(false)
        .disable_help_subcommand(false)
        .propagate_version(false);
    for (name, kind, usage, default) in GLOBAL_FLAGS {
        root = root.arg(flag_arg(name, None, *kind, usage, *default).global(true));
    }
    let jini = jini_enabled();
    for c in crate::spec::COMMANDS {
        if c.experimental && !jini {
            continue;
        }
        root = root.subcommand(command(c, c.name, jini));
    }
    let mut agent_context = Command::new("agent-context").about("Emit structured JSON describing this CLI for agents");
    if let Some((long, example)) = crate::help_text::lookup("agent-context", jini) {
        if !long.is_empty() {
            agent_context = agent_context.long_about(long);
        }
        if !example.is_empty() {
            agent_context = agent_context.after_help(format!("Examples:\n{example}"));
        }
    }
    root.subcommand(
        agent_context.arg(
            Arg::new("pretty")
                .long("pretty")
                .action(ArgAction::SetTrue)
                .help("indent JSON output for human reading"),
        ),
    )
}

fn command(c: &'static Cmd, path: &str, jini: bool) -> Command {
    let mut cmd = Command::new(c.name).about(c.short).hide(c.hidden);
    if let Some((long, example)) = crate::help_text::lookup(path, jini).or_else(|| crate::help_extra::lookup(path)) {
        if !long.is_empty() {
            cmd = cmd.long_about(long);
        }
        if !example.is_empty() {
            cmd = cmd.after_help(format!("Examples:\n{example}"));
        }
    }
    // Cobra's persistent --db belongs to these groups and is accepted on
    // their children; it is intentionally absent from the command snapshot.
    if matches!(path, "shield dict" | "vault") {
        cmd = cmd.arg(Arg::new("db").long("db").global(true).help("SQLite database file path"));
    }
    for f in c.flags {
        let (usage, hidden) = crate::spec_types::flag_presentation(path, f, jini);
        cmd = cmd.arg(flag_arg(f.name, f.short, f.kind, usage, f.default).hide(hidden));
    }
    let positionals = c.positionals();
    for (i, p) in positionals.iter().enumerate() {
        let mut arg = Arg::new(p.name.clone()).value_name(p.name.clone()).index(i + 1);
        // Go commands validate their own arguments and print command help
        // when one is missing, so none are required at the parser level.
        arg = if p.variadic {
            arg.num_args(0..).action(ArgAction::Append)
        } else {
            arg.required(false)
        };
        if path == "keys mint" {
            arg = arg.last(true).allow_hyphen_values(true);
        }
        cmd = cmd.arg(arg);
    }
    if c.subcommands.is_empty() && !positionals.iter().any(|p| p.variadic) {
        // cobra accepts extra trailing words; `which compare models` joins them.
        cmd = cmd.arg(
            Arg::new(EXTRA_ARGS)
                .index(positionals.len() + 1)
                .num_args(0..)
                .action(ArgAction::Append)
                .hide(true),
        );
    }
    for s in c.subcommands {
        cmd = cmd.subcommand(command(s, &format!("{path} {}", s.name), jini));
    }
    cmd
}

fn flag_arg(name: &str, short: Option<char>, kind: Kind, usage: &str, default: Option<&str>) -> Arg {
    let mut arg = Arg::new(name.to_string())
        .long(name.to_string())
        .help(usage.to_string());
    if let Some(s) = short {
        arg = arg.short(s);
    }
    match kind {
        Kind::Bool => {
            // Go bool flags also accept --flag=false.
            arg = arg
                .num_args(0..=1)
                .require_equals(true)
                .default_missing_value("true")
                .value_parser(clap::builder::BoolishValueParser::new())
                .action(ArgAction::Set);
        }
        Kind::StrSlice => {
            arg = arg.action(ArgAction::Append).value_delimiter(',');
        }
        Kind::StrArray => {
            arg = arg.action(ArgAction::Append);
        }
        _ => {
            arg = arg.action(ArgAction::Set);
        }
    }
    if let Some(d) = default
        && kind != Kind::Bool
        && !d.is_empty()
        && d != "[]"
    {
        arg = arg.default_value(d.to_string());
    }
    arg
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tree_is_valid() {
        build().debug_assert();
    }

    #[test]
    fn parses_global_and_local_flags() {
        let m = build()
            .try_get_matches_from([
                "mxl",
                "--agent",
                "shield",
                "redact",
                "x.csv",
                "-o",
                "out.csv",
                "--mask-columns",
                "a,b",
            ])
            .unwrap();
        let (_, sm) = m.subcommand().unwrap();
        let (_, rm) = sm.subcommand().unwrap();
        assert_eq!(rm.get_one::<String>("output").map(String::as_str), Some("out.csv"));
        let cols: Vec<_> = rm.get_many::<String>("mask-columns").unwrap().cloned().collect();
        assert_eq!(cols, ["a", "b"]);
        assert_eq!(rm.get_one::<bool>("agent"), Some(&true));
    }
}
