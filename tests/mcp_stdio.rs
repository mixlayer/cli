// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

use std::io::Write;
use std::process::{Command, Stdio};

use serde_json::{Value, json};

#[test]
fn stdio_tools_match_go_and_sql_blocks_vault() {
    let home = tempfile::tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_mixlayer-mcp"))
        .env("HOME", home.path())
        .env("MIXLAYER_HOME", home.path())
        .env("MIXLAYER_ENABLE_JINI", "0")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let requests = [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"parity-test","version":"1"}}}),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}),
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"sql","arguments":{"query":"select value from vault"}}}),
        json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"ask","arguments":{"question":"hello","dry_run":true}}}),
    ];
    {
        let stdin = child.stdin.as_mut().unwrap();
        for request in requests {
            writeln!(stdin, "{request}").unwrap();
        }
    }
    drop(child.stdin.take());
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let messages: Vec<Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let response = |id: u64| messages.iter().find(|m| m.get("id") == Some(&json!(id))).unwrap();
    let go: Value = serde_json::from_str(include_str!("../parity/fixtures/go-mcp-tools.json")).unwrap();
    let init = &response(1)["result"];
    // Same server name as Go; the version tracks Cargo.toml (Go's snapshot is from 0.1.0-dev).
    assert_eq!(init["serverInfo"]["name"], go["initialize"]["serverInfo"]["name"]);
    assert_eq!(init["serverInfo"]["version"], env!("CARGO_PKG_VERSION"));
    let pick = |tools: &Value| -> Vec<Value> {
        let mut list: Vec<Value> = tools.as_array().unwrap().iter().map(|tool| {
            json!({"name":tool["name"],"description":tool["description"],"inputSchema":tool["inputSchema"],"annotations":tool["annotations"]})
        }).collect();
        list.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
        list
    };
    let mut rust_tools = pick(&response(2)["result"]["tools"]);
    for name in ["whoami", "spend"] {
        assert!(rust_tools.iter().any(|tool| tool["name"] == name));
    }
    rust_tools.retain(|tool| tool["name"] != "whoami" && tool["name"] != "spend");
    assert_eq!(rust_tools, pick(&go["tools"]));
    let sql = &response(3)["result"];
    assert_eq!(sql["isError"], true, "{sql}");
    assert!(sql["content"][0]["text"].as_str().unwrap().contains("vault"));
    let ask = &response(4)["result"];
    assert_eq!(ask["isError"], false, "{ask}");
    let preview: Value = serde_json::from_str(ask["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(preview["dry_run"], true);
    assert_eq!(preview["messages"][0]["content"], "hello");
}
