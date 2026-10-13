// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

//! Parity with the Go CLI: `agent-context` must describe the same commands,
//! flags, defaults, annotations and exit codes as the Go build it replaces.
//! Snapshots live in parity/ and were produced by the Go CLI.

use std::process::Command;

use serde_json::Value;

fn rust_context(jini: bool) -> Value {
    let home = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mxl"))
        .arg("agent-context")
        .env("MIXLAYER_HOME", home.path())
        .env("MIXLAYER_ENABLE_JINI", if jini { "1" } else { "0" })
        .env_remove("AI_AGENT")
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    serde_json::from_slice(&out.stdout).unwrap()
}

fn go_snapshot(name: &str) -> Value {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/parity/");
    serde_json::from_str(&std::fs::read_to_string(format!("{path}{name}")).unwrap()).unwrap()
}

/// Drops values that depend on the machine (resolved paths).
fn normalize(mut v: Value) -> Value {
    v["paths"] = Value::Null;
    // The Go snapshots were taken from a 0.1.0-dev build; the version tracks Cargo.toml.
    v["cli"]["version"] = Value::Null;
    v
}

fn diff(go: &Value, rs: &Value, path: &str, out: &mut Vec<String>) {
    match (go, rs) {
        (Value::Object(a), Value::Object(b)) => {
            for k in a.keys().chain(b.keys().filter(|k| !a.contains_key(*k))) {
                match (a.get(k), b.get(k)) {
                    (Some(x), Some(y)) => diff(x, y, &format!("{path}.{k}"), out),
                    (Some(_), None) => out.push(format!("{path}.{k}: missing in Rust")),
                    (None, Some(_)) => out.push(format!("{path}.{k}: only in Rust")),
                    (None, None) => unreachable!(),
                }
            }
        }
        (Value::Array(a), Value::Array(b)) => {
            if a.len() != b.len() {
                out.push(format!("{path}: {} items in Go, {} in Rust", a.len(), b.len()));
            }
            for (i, (x, y)) in a.iter().zip(b).enumerate() {
                let label = x
                    .get("name")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or(i.to_string());
                diff(x, y, &format!("{path}[{label}]"), out);
            }
        }
        (x, y) if x != y => out.push(format!("{path}: Go {x} vs Rust {y}")),
        _ => {}
    }
}

/// One intentional difference from Go: with Jini off, flags that `--help`
/// hides (they describe the experimental Jini checker) are also left out of
/// agent-context. Go listed them with Jini wording.
fn drop_hidden_jini_flags(v: &mut Value) {
    for (cmd, flag, usage) in mixlayer::spec_types::JINI_OFF_FLAGS {
        if usage.is_some() {
            continue;
        }
        let Some(commands) = v["commands"].as_array_mut() else {
            continue;
        };
        for c in commands.iter_mut().filter(|c| c["name"] == *cmd) {
            if let Some(flags) = c["flags"].as_array_mut() {
                flags.retain(|f| f["name"] != *flag);
            }
        }
    }
}

/// Removes commands and flags added after the Go port (see
/// `spec_types::RUST_ONLY`) so the rest is still held to the Go snapshot.
fn drop_rust_only(v: &mut Value) {
    for (path, flag) in mixlayer::spec_types::RUST_ONLY {
        let parts: Vec<&str> = path.split_whitespace().collect();
        let (last, parents) = parts.split_last().unwrap();
        let mut list = &mut v["commands"];
        for p in parents {
            let Some(cmd) = list.as_array_mut().and_then(|a| a.iter_mut().find(|c| c["name"] == *p)) else {
                panic!("RUST_ONLY entry {path:?} names a missing parent {p:?}");
            };
            list = &mut cmd["subcommands"];
        }
        let commands = list.as_array_mut().expect("command list");
        match flag {
            None => {
                let before = commands.len();
                commands.retain(|c| c["name"] != *last);
                assert_eq!(before - 1, commands.len(), "RUST_ONLY command {path:?} not found");
            }
            Some(flag) => {
                let cmd = commands
                    .iter_mut()
                    .find(|c| c["name"] == *last)
                    .unwrap_or_else(|| panic!("RUST_ONLY command {path:?} not found"));
                let flags = cmd["flags"].as_array_mut().expect("flags");
                let before = flags.len();
                flags.retain(|f| f["name"] != *flag);
                assert_eq!(before - 1, flags.len(), "RUST_ONLY flag {path:?} --{flag} not found");
            }
        }
    }
}

fn assert_parity(snapshot: &str, jini: bool) {
    let mut go = normalize(go_snapshot(snapshot));
    if !jini {
        drop_hidden_jini_flags(&mut go);
    }
    let mut rs = normalize(rust_context(jini));
    drop_rust_only(&mut rs);
    let mut out = Vec::new();
    diff(&go, &rs, "$", &mut out);
    assert!(
        out.is_empty(),
        "{} differences from {snapshot}:\n{}",
        out.len(),
        out.join("\n")
    );
}

#[test]
fn agent_context_matches_go() {
    assert_parity("go-agent-context.json", false);
}

#[test]
fn agent_context_matches_go_with_jini() {
    assert_parity("go-agent-context-jini.json", true);
}
