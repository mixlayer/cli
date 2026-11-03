// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.
//! Login and billing commands never prompt without a terminal and fail fast on bad input.

use std::io::Write;
use std::process::{Command, Stdio};

fn mxl(args: &[&str], stdin: &str) -> (i32, String, String) {
    let home = tempfile::tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_mxl"))
        .args(args)
        .env("HOME", home.path())
        .env("XDG_CONFIG_HOME", home.path())
        .env("MIXLAYER_NO_AGENT_DETECT", "1")
        .env("MIXLAYER_API_BASE", "http://127.0.0.1:9")
        .env("MIXLAYER_PLATFORM_BASE", "http://127.0.0.1:9")
        .env_remove("MIXLAYER_API_KEY")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
    let o = child.wait_with_output().unwrap();
    (
        o.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&o.stdout).into(),
        String::from_utf8_lossy(&o.stderr).into(),
    )
}

#[test]
fn login_without_terminal_refuses_with_hint() {
    let (code, _, err) = mxl(&["login"], "");
    assert_eq!(code, 2);
    assert!(err.contains("--with-token"), "{err}");
}

#[test]
fn with_token_rejects_junk_without_echoing_it() {
    let (code, out, err) = mxl(&["login", "--with-token"], "not a key\n");
    assert_eq!(code, 2);
    assert!(!out.contains("not a key") && !err.contains("not a key"));
}

#[test]
fn add_credit_bad_amount_is_usage_error_before_network() {
    for amt in ["0", "5000", "abc"] {
        let (code, _, _) = mxl(&["billing", "add-credit", "--amount", amt], "");
        assert_eq!(code, 2, "{amt}");
    }
}

#[test]
fn login_dry_run_is_json_and_secret_free() {
    let (code, out, _) = mxl(&["login", "--dry-run"], "");
    assert_eq!(code, 0);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["dry_run"], true);
}
