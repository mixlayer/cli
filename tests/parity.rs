// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

//! Parity with the Go CLI: `agent-context` must describe the same commands,
//! flags, defaults, annotations and exit codes as the Go build it replaces.
//! Snapshots live in parity/ and were produced by the Go CLI.

use std::process::Command;

use serde_json::Value;

fn rust_context(jini: bool) -> Value {
    let home = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mixlayer"))
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

fn assert_parity(snapshot: &str, jini: bool) {
    let mut go = normalize(go_snapshot(snapshot));
    if !jini {
        drop_hidden_jini_flags(&mut go);
    }
    let rs = normalize(rust_context(jini));
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
