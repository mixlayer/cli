// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

//! `mxl feedback` builds links only, and failures never prompt without a TTY.

use std::process::Command;

fn mxl(home: &std::path::Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_mxl"));
    c.env("MIXLAYER_HOME", home)
        .env_remove("MXL_FEEDBACK")
        .env_remove("AI_AGENT")
        .env("MIXLAYER_NO_AGENT_DETECT", "1");
    c
}

#[test]
fn dry_run_prints_scrubbed_report_and_sends_nothing() {
    let home = tempfile::tempdir().unwrap();
    let out = mxl(home.path())
        .args([
            "feedback",
            "--dry-run",
            "--kind",
            "confusing",
            "--command",
            "models list",
            "my key sk-abcdefghijklmnop lives in /Users/zed/.env, mail zed@example.com",
        ])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("nothing is sent"));
    assert!(s.contains("https://github.com/mixlayer/cli/issues/new?"));
    assert!(s.contains("[mxl-feedback]") && s.contains("automated"));
    assert!(s.contains("kind:confusing") && s.contains("cmd:models-list"));
    for leak in ["sk-abcdefghijklmnop", "/Users/zed", "zed@example.com"] {
        assert!(!s.contains(leak), "{leak} leaked:\n{s}");
    }
}

#[test]
fn json_output_has_issue_url_and_fingerprint() {
    let home = tempfile::tempdir().unwrap();
    let out = mxl(home.path())
        .args(["feedback", "--json", "--dry-run", "slow start"])
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(
        v["issue_url"]
            .as_str()
            .unwrap()
            .starts_with("https://github.com/mixlayer/cli/issues/new?")
    );
    assert!(v["fingerprint"].as_str().unwrap().starts_with("mxl-"));
}

#[test]
fn never_mode_refuses() {
    let home = tempfile::tempdir().unwrap();
    let out = mxl(home.path())
        .env("MXL_FEEDBACK", "never")
        .args(["feedback", "x"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(10));
}

#[test]
fn failure_without_tty_does_not_offer_or_change_exit_code() {
    let home = tempfile::tempdir().unwrap();
    let out = mxl(home.path())
        .args(["which", "zzzz-no-such-capability"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(!err.contains("issues/new") && !err.contains("[y/N]"), "{err}");
}
