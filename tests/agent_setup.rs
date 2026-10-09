// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

fn executable(path: &Path, text: &str) {
    fs::write(path, text).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }
}

fn setup(
    home: &Path,
    mcp: &Path,
    agent: &str,
    yes: bool,
    uninstall: bool,
    path: Option<&Path>,
) -> Result<Value, String> {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_mixlayer"));
    cmd.args(["agent", "setup", "--agent", agent])
        .env("HOME", home)
        .env("MIXLAYER_MCP_PATH", mcp)
        .env("CLAUDE_CONFIG_DIR", home.join(".claude"))
        .env("MIXLAYER_API_KEY", "synthetic-secret-never-write-8472")
        .env("MIXLAYER_NO_AGENT_DETECT", "1");
    if let Some(dir) = path {
        cmd.env(
            "PATH",
            format!("{}:{}", dir.display(), std::env::var("PATH").unwrap_or_default()),
        );
    }
    if yes {
        cmd.arg("--yes");
    }
    if uninstall {
        cmd.arg("--uninstall");
    }
    let output = cmd.output().unwrap();
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }
    serde_json::from_slice(&output.stdout).map_err(|e| e.to_string())
}

fn statuses(report: &Value) -> Vec<&str> {
    report["actions"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|a| a["status"].as_str())
        .collect()
}

fn skill_path(home: &Path, agent: &str) -> Option<PathBuf> {
    let rel = match agent {
        "claude-code" => ".claude/skills/mixlayer",
        "codex" => ".codex/skills/mixlayer",
        "cursor" => ".cursor/skills/mixlayer",
        "hermes" => ".hermes/skills/mixlayer",
        _ => return None,
    };
    Some(home.join(rel))
}

fn no_key_in_tree(path: &Path) {
    if !path.exists() {
        return;
    }
    for item in fs::read_dir(path).unwrap() {
        let item = item.unwrap();
        if item.file_type().unwrap().is_dir() {
            no_key_in_tree(&item.path());
        } else {
            let bytes = fs::read(item.path()).unwrap();
            assert!(!String::from_utf8_lossy(&bytes).contains("synthetic-secret-never-write-8472"));
        }
    }
}

#[test]
fn each_agent_preview_install_idempotent_uninstall() {
    for agent in ["claude-code", "codex", "cursor", "hermes", "claude-desktop"] {
        let home = tempfile::tempdir().unwrap();
        let binaries = tempfile::tempdir().unwrap();
        let mcp = binaries.path().join("mixlayer-mcp");
        executable(&mcp, "#!/bin/sh\nexit 0\n");
        executable(
            &binaries.path().join("claude"),
            "#!/bin/sh\nconfig=\"$CLAUDE_CONFIG_DIR/.claude.json\"\nif [ \"$2\" = add ]; then printf '{\"mcpServers\":{\"mixlayer\":{\"command\":\"fake\"}}}\\n' > \"$config\"; else printf '{}\\n' > \"$config\"; fi\n",
        );
        let preview = setup(home.path(), &mcp, agent, false, false, Some(binaries.path())).unwrap();
        assert_eq!(preview["dry_run"], true);
        assert!(!statuses(&preview).is_empty());
        assert_eq!(
            fs::read_dir(home.path()).unwrap().count(),
            0,
            "{agent} preview wrote HOME"
        );
        if agent == "claude-code" {
            fs::create_dir_all(home.path().join(".claude")).unwrap();
            fs::write(home.path().join(".claude/.claude.json"), "{}\n").unwrap();
        }
        let installed = setup(home.path(), &mcp, agent, true, false, Some(binaries.path())).unwrap();
        if let Some(skill) = skill_path(home.path(), agent) {
            assert!(skill.join("SKILL.md").exists(), "{agent}");
        }
        if agent != "hermes" {
            assert!(statuses(&installed).contains(&"registered"), "{agent}: {installed}");
        }
        if agent == "claude-code" {
            assert!(fs::read_dir(home.path().join(".claude")).unwrap().any(|e| {
                e.unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".claude.json.bak-")
            }));
        }
        no_key_in_tree(home.path());
        let repeated = setup(home.path(), &mcp, agent, true, false, Some(binaries.path())).unwrap();
        assert!(
            !statuses(&repeated)
                .iter()
                .any(|v| ["installed", "registered", "conflict_skip"].contains(v)),
            "{agent}: {repeated}"
        );
        let removed = setup(home.path(), &mcp, agent, true, true, Some(binaries.path())).unwrap();
        assert!(statuses(&removed).contains(&"removed"), "{agent}: {removed}");
        if let Some(skill) = skill_path(home.path(), agent) {
            assert!(!skill.exists(), "{agent}");
        }
        no_key_in_tree(home.path());
    }
}

#[test]
fn restores_preexisting_skill_and_rejects_missing_mcp() {
    let home = tempfile::tempdir().unwrap();
    let skill = skill_path(home.path(), "codex").unwrap();
    fs::create_dir_all(&skill).unwrap();
    fs::write(skill.join("SKILL.md"), "original user skill\n").unwrap();
    fs::write(skill.join("user.txt"), "keep").unwrap();
    let missing = home.path().join("missing-mcp");
    assert!(setup(home.path(), &missing, "codex", true, false, None).is_err());
    assert_eq!(
        fs::read_to_string(skill.join("SKILL.md")).unwrap(),
        "original user skill\n"
    );
    let binaries = tempfile::tempdir().unwrap();
    let mcp = binaries.path().join("mixlayer-mcp");
    executable(&mcp, "#!/bin/sh\nexit 0\n");
    setup(home.path(), &mcp, "codex", true, false, None).unwrap();
    setup(home.path(), &mcp, "codex", true, true, None).unwrap();
    assert_eq!(
        fs::read_to_string(skill.join("SKILL.md")).unwrap(),
        "original user skill\n"
    );
    assert!(skill.join("user.txt").exists());
}

#[test]
fn preserves_unrelated_json_and_backs_up_before_edit() {
    let home = tempfile::tempdir().unwrap();
    let binaries = tempfile::tempdir().unwrap();
    let mcp = binaries.path().join("mixlayer-mcp");
    executable(&mcp, "#!/bin/sh\nexit 0\n");
    let config = home.path().join(".cursor/mcp.json");
    fs::create_dir_all(config.parent().unwrap()).unwrap();
    fs::write(
        &config,
        "{\"mcpServers\":{\"other\":{\"command\":\"other-bin\"}},\"custom\":true}\n",
    )
    .unwrap();
    setup(home.path(), &mcp, "cursor", true, false, None).unwrap();
    assert!(
        fs::read_dir(config.parent().unwrap()).unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("mcp.json.bak-"))
    );
    setup(home.path(), &mcp, "cursor", true, true, None).unwrap();
    let doc: Value = serde_json::from_slice(&fs::read(config).unwrap()).unwrap();
    assert_eq!(doc["custom"], true);
    assert_eq!(doc["mcpServers"]["other"]["command"], "other-bin");
    assert!(doc["mcpServers"].get("mixlayer").is_none());
}

#[test]
fn codex_toml_merge_preserves_original_bytes() {
    let home = tempfile::tempdir().unwrap();
    let binaries = tempfile::tempdir().unwrap();
    let mcp = binaries.path().join("mixlayer-mcp");
    executable(&mcp, "#!/bin/sh\nexit 0\n");
    let config = home.path().join(".codex/config.toml");
    fs::create_dir_all(config.parent().unwrap()).unwrap();
    let original = b"# user's comment\n[other]\nmode = 'keep'\n";
    fs::write(&config, original).unwrap();
    setup(home.path(), &mcp, "codex", true, false, None).unwrap();
    assert!(
        fs::read_dir(config.parent().unwrap()).unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("config.toml.bak-"))
    );
    setup(home.path(), &mcp, "codex", true, true, None).unwrap();
    assert_eq!(fs::read(config).unwrap(), original);
}

#[test]
fn claude_modified_entry_is_not_removed() {
    let home = tempfile::tempdir().unwrap();
    let binaries = tempfile::tempdir().unwrap();
    let mcp = binaries.path().join("mixlayer-mcp");
    executable(&mcp, "#!/bin/sh\nexit 0\n");
    executable(
        &binaries.path().join("claude"),
        "#!/bin/sh\nconfig=\"$CLAUDE_CONFIG_DIR/.claude.json\"\nif [ \"$2\" = add ]; then printf '{\"mcpServers\":{\"mixlayer\":{\"command\":\"owned\"}}}\\n' > \"$config\"; else printf '{}\\n' > \"$config\"; fi\n",
    );
    setup(home.path(), &mcp, "claude-code", true, false, Some(binaries.path())).unwrap();
    let config = home.path().join(".claude/.claude.json");
    let changed = b"{\"mcpServers\":{\"mixlayer\":{\"command\":\"user-replaced\"}}}\n";
    fs::write(&config, changed).unwrap();
    let report = setup(home.path(), &mcp, "claude-code", true, true, Some(binaries.path())).unwrap();
    assert!(statuses(&report).contains(&"modified_skip"));
    assert_eq!(fs::read(config).unwrap(), changed);
}

#[test]
fn setup_requires_true_explicit_yes() {
    let home = tempfile::tempdir().unwrap();
    let binaries = tempfile::tempdir().unwrap();
    let mcp = binaries.path().join("mixlayer-mcp");
    executable(&mcp, "#!/bin/sh\nexit 0\n");
    for yes in [None, Some("--yes=false")] {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_mixlayer"));
        cmd.args(["agent", "setup", "--agent", "codex"])
            .env("HOME", home.path())
            .env("MIXLAYER_MCP_PATH", &mcp);
        if let Some(arg) = yes {
            cmd.arg(arg);
        }
        let output = cmd.output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let report: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(report["dry_run"], true);
        assert_eq!(fs::read_dir(home.path()).unwrap().count(), 0);
    }
}
