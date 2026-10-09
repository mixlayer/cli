// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

//! `agent-context`: the CLI's own description as JSON, so agents can learn
//! every command, flag and exit code without parsing --help. The shape
//! (schema_version 4) matches the Go CLI field for field.

use serde_json::{Map, Value, json};

use crate::spec_types::Cmd;
use crate::{cli, exit, paths, profiles};

pub const SCHEMA_VERSION: &str = "4";

pub fn build() -> Value {
    let jini = cli::jini_enabled();
    let dir = |k| paths::dir(k).map(|p| p.display().to_string()).unwrap_or_default();
    json!({
        "schema_version": SCHEMA_VERSION,
        "cli": {"name": "mixlayer", "description": "CLI for mixlayer", "version": cli::VERSION},
        "auth": {
            "mode": "bearer_token",
            "env_vars": [{
                "name": "MIXLAYER_API_KEY",
                "kind": "per_call",
                "required": true,
                "sensitive": true,
                "description": "Set to your API credential."
            }]
        },
        "exit_codes": exit::TABLE.iter().map(|(c, m)| json!({"code": *c as u8, "meaning": m})).collect::<Vec<_>>(),
        "paths": {
            "config_dir": dir(paths::Kind::Config),
            "data_dir": dir(paths::Kind::Data),
            "state_dir": dir(paths::Kind::State),
            "cache_dir": dir(paths::Kind::Cache),
        },
        "commands": commands(crate::spec::COMMANDS, "", jini, true),
        "available_profiles": profiles::names(),
    })
}

fn commands(list: &'static [Cmd], parent: &str, jini: bool, top: bool) -> Vec<Value> {
    let mut cmds: Vec<&Cmd> = list.iter().filter(|c| jini || !c.experimental).collect();
    cmds.sort_by_key(|c| c.name);
    let mut out: Vec<Value> = cmds
        .into_iter()
        .map(|c| command(c, format!("{parent} {}", c.name).trim(), jini))
        .collect();
    if top {
        // cobra adds `help` at the root; keep it so the listing matches.
        out.push(help_entry());
        out.sort_by_key(|v| v["name"].as_str().unwrap_or_default().to_string());
    }
    out
}

fn command(c: &'static Cmd, path: &str, jini: bool) -> Value {
    let mut m = Map::new();
    m.insert("name".into(), c.name.into());
    m.insert("use".into(), c.usage.into());
    if !c.short.is_empty() {
        m.insert("short".into(), c.short.into());
    }
    if !c.annotations.is_empty() {
        let mut ann: Vec<_> = c.annotations.to_vec();
        ann.sort();
        m.insert(
            "annotations".into(),
            Value::Object(ann.into_iter().map(|(k, v)| (k.to_string(), Value::from(v))).collect()),
        );
    }
    if !c.flags.is_empty() {
        // Experimental flags hidden from --help stay out of agent discovery
        // too, so nothing names the Jini surface unless it is enabled.
        let mut flags: Vec<_> = c
            .flags
            .iter()
            .filter(|f| !crate::spec_types::flag_presentation(path, f, jini).1)
            .collect();
        flags.sort_by_key(|f| f.name);
        m.insert(
            "flags".into(),
            flags
                .into_iter()
                .map(|f| {
                    let mut fm = Map::new();
                    fm.insert("name".into(), f.name.into());
                    fm.insert("type".into(), f.kind.go_name().into());
                    let (usage, _) = crate::spec_types::flag_presentation(path, f, jini);
                    if !usage.is_empty() {
                        fm.insert("usage".into(), usage.into());
                    }
                    if let Some(d) = f.default
                        && !d.is_empty()
                    {
                        fm.insert("default".into(), d.into());
                    }
                    Value::Object(fm)
                })
                .collect(),
        );
    }
    if !c.subcommands.is_empty() {
        m.insert("subcommands".into(), commands(c.subcommands, path, jini, false).into());
    }
    Value::Object(m)
}

fn help_entry() -> Value {
    json!({"name": "help", "use": "help [command]", "short": "Help about any command"})
}
