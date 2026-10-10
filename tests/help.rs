// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

//! Help text: every entry in the generated help table belongs to a real
//! command, and no help page mentions the experimental Jini surface unless
//! `MIXLAYER_ENABLE_JINI=1`.

use std::process::Command;

use mixlayer::spec_types::Cmd;

fn paths(cmds: &'static [Cmd], prefix: &str, out: &mut Vec<(String, bool)>) {
    for c in cmds {
        let p = if prefix.is_empty() {
            c.name.to_string()
        } else {
            format!("{prefix} {}", c.name)
        };
        out.push((p.clone(), c.experimental));
        paths(c.subcommands, &p, out);
    }
}

fn all_paths() -> Vec<(String, bool)> {
    let mut out = Vec::new();
    paths(mixlayer::spec::COMMANDS, "", &mut out);
    out
}

#[test]
fn help_table_entries_are_real_commands() {
    let mut known: Vec<String> = all_paths().into_iter().map(|(p, _)| p).collect();
    known.push("agent-context".into()); // built outside the command table
    for (p, _, _) in mixlayer::help_text::HELP
        .iter()
        .chain(mixlayer::help_text::HELP_JINI)
        .chain(mixlayer::help_extra::HELP)
    {
        assert!(known.iter().any(|k| k == p), "help entry for unknown command {p:?}");
    }
}

#[test]
fn help_never_mentions_jini_when_disabled() {
    let bin = env!("CARGO_BIN_EXE_mixlayer");
    let mut checked = 0;
    let extra = [(String::new(), false), ("agent-context".to_string(), false)];
    for (p, experimental) in extra.into_iter().chain(all_paths()) {
        if experimental {
            continue;
        }
        let out = Command::new(bin)
            .args(p.split_whitespace())
            .arg("--help")
            .env_remove("MIXLAYER_ENABLE_JINI")
            .output()
            .expect("run mixlayer");
        let text = String::from_utf8_lossy(&out.stdout).to_lowercase();
        for banned in ["jini", "systemone", "noul"] {
            assert!(!text.contains(banned), "`mixlayer {p} --help` mentions {banned:?}");
        }
        checked += 1;
    }
    assert!(checked > 40, "only {checked} help pages checked");
}
