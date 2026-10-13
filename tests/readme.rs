// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.
//! Every `mxl ...` command shown in README.md must parse against the real
//! command tree, so the README can't drift from the CLI.

use std::path::Path;

/// Shell-style split that understands single and double quotes. Stops at an
/// unquoted pipe, redirect or comment, which belong to the shell, not to mxl.
fn split(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut started = false;
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => cur.push(c),
            None if c == '\'' || c == '"' => {
                quote = Some(c);
                started = true;
            }
            None if c == '|' || c == '>' || c == '#' => break,
            None if c.is_whitespace() => {
                if started {
                    out.push(std::mem::take(&mut cur));
                    started = false;
                }
            }
            None => {
                cur.push(c);
                started = true;
            }
        }
        i += 1;
    }
    if started {
        out.push(cur);
    }
    out
}

fn readme_commands() -> Vec<String> {
    let text = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("README.md")).unwrap();
    let mut cmds = Vec::new();
    let mut in_sh = false;
    let mut pending = String::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            in_sh = !in_sh && (trimmed == "```sh" || trimmed == "```bash");
            continue;
        }
        if in_sh {
            let joined = format!("{pending}{trimmed}");
            if let Some(rest) = joined.strip_suffix('\\') {
                pending = format!("{rest} ");
                continue;
            }
            pending.clear();
            // `pbpaste | mxl auth set-token`: the mxl part follows the pipe.
            if let Some(pos) = joined.find("mxl ")
                && (pos == 0 || joined[..pos].ends_with("| "))
            {
                cmds.push(joined[pos..].to_string());
            }
        } else {
            for (n, span) in line.split('`').enumerate() {
                if n % 2 == 1 && span.starts_with("mxl ") {
                    cmds.push(span.to_string());
                }
            }
        }
    }
    cmds
}

#[test]
fn every_readme_command_parses() {
    let cmds = readme_commands();
    assert!(cmds.len() > 40, "found only {} commands", cmds.len());
    let mut failures = Vec::new();
    for cmd in &cmds {
        let args: Vec<String> = split(cmd)
            .into_iter()
            .map(|a| {
                if a.starts_with('<') && a.ends_with('>') {
                    "x".into()
                } else {
                    a
                }
            })
            .collect();
        if let Err(e) = mixlayer::cli::build().try_get_matches_from(&args) {
            failures.push(format!(
                "{cmd}\n    {}",
                e.to_string().lines().next().unwrap_or_default()
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "README commands that don't parse:\n{}",
        failures.join("\n")
    );
}
