// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

//! Install the embedded skill and register the MCP binary without ever
//! handling the user's API key.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

const ORDER: &[&str] = &["claude-code", "codex", "cursor", "hermes", "claude-desktop"];

#[derive(Clone, Debug)]
pub struct Options {
    pub agent: String,
    pub mcp_binary: Option<PathBuf>,
    pub dry_run: bool,
    pub uninstall: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct Action {
    pub agent: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Report {
    pub dry_run: bool,
    pub uninstall: bool,
    pub actions: Vec<Action>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct Entry {
    #[serde(default, skip_serializing_if = "is_false")]
    skill_installed: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    skill_backup: String,
    #[serde(default, skip_serializing_if = "is_false")]
    mcp_installed: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    mcp_file_created: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mcp_value: Option<Value>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    mcp_hash: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    mcp_snippet: String,
}

fn is_false(v: &bool) -> bool {
    !*v
}

pub fn targets(name: &str) -> Result<Vec<&'static str>> {
    if name.is_empty() || name == "all" {
        return Ok(ORDER.to_vec());
    }
    ORDER
        .iter()
        .copied()
        .find(|v| *v == name)
        .map(|v| vec![v])
        .ok_or_else(|| {
            anyhow::anyhow!(
                "unknown agent {name:?} (choose claude-code, codex, cursor, hermes, claude-desktop, or all)"
            )
        })
}

pub fn mcp_binary() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("MIXLAYER_MCP_PATH").filter(|v| !v.is_empty()) {
        let p = PathBuf::from(path);
        return Some(if p.is_absolute() {
            p
        } else {
            std::env::current_dir().ok()?.join(p)
        });
    }
    let name = if cfg!(windows) {
        "mixlayer-mcp.exe"
    } else {
        "mixlayer-mcp"
    };
    if let Ok(exe) = std::env::current_exe() {
        let sibling = exe.with_file_name(name);
        if sibling.is_file() {
            return Some(sibling);
        }
    }
    find_executable(name)
}

fn find_executable(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(name))
        .find(|p| executable(p))
}

fn executable(path: &Path) -> bool {
    let Ok(meta) = fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

pub fn run(mut opts: Options) -> Result<Report> {
    let targets = targets(&opts.agent)?;
    if !opts.dry_run && !opts.uninstall && targets.iter().any(|a| *a != "hermes") {
        let bin = opts.mcp_binary.clone().or_else(mcp_binary);
        if !bin.as_deref().is_some_and(executable) {
            bail!("mixlayer-mcp executable not found; build or install it beside mixlayer before setup");
        }
        opts.mcp_binary = bin;
    }
    let home = home()?;
    let state_path = home.join(".config/mixlayer/agent-setup.json");
    let mut state: BTreeMap<String, Entry> = if state_path.exists() {
        serde_json::from_slice(&fs::read(&state_path)?).context("reading setup state")?
    } else {
        BTreeMap::new()
    };
    let mut report = Report {
        dry_run: opts.dry_run,
        uninstall: opts.uninstall,
        actions: Vec::new(),
    };
    let mut changed = false;
    for agent in targets {
        let mut rec = state.get(agent).cloned().unwrap_or_default();
        let previous = rec.clone();
        let result = (|| -> Result<()> {
            if let Some(rel) = skill_location(agent) {
                // Join per component so Windows paths use one separator.
                let dir = rel.split('/').fold(home.clone(), |p, part| p.join(part));
                changed |= skill_action(&mut report, agent, &dir, &mut rec, &opts)?;
            }
            changed |= mcp_action(&mut report, agent, &home, &mut rec, &opts)?;
            Ok(())
        })();
        if !opts.dry_run {
            if rec == Entry::default() {
                state.remove(agent);
            } else {
                state.insert(agent.to_owned(), rec);
            }
        }
        if let Err(error) = result {
            if !opts.dry_run && (changed || state.get(agent) != Some(&previous)) {
                save_state(&state_path, &state)?;
            }
            return Err(error);
        }
    }
    if changed && !opts.dry_run {
        if state.is_empty() {
            if state_path.exists() {
                backup_file(&state_path)?;
                fs::remove_file(&state_path)?;
            }
        } else {
            save_state(&state_path, &state)?;
        }
    }
    Ok(report)
}

fn home() -> Result<PathBuf> {
    // HOME on Unix, USERPROFILE on Windows (like Go's os.UserHomeDir).
    crate::paths::user_home().ok_or_else(|| anyhow::anyhow!("resolve user home dir: not found"))
}

fn skill_location(agent: &str) -> Option<&'static str> {
    match agent {
        "claude-code" => Some(".claude/skills/mixlayer"),
        "codex" => Some(".codex/skills/mixlayer"),
        "cursor" => Some(".cursor/skills/mixlayer"),
        "hermes" => Some(".hermes/skills/mixlayer"),
        _ => None,
    }
}

fn action(
    report: &mut Report,
    agent: &str,
    kind: &str,
    path: Option<&Path>,
    status: &str,
    note: Option<&str>,
    command: Option<String>,
) {
    report.actions.push(Action {
        agent: agent.into(),
        kind: kind.into(),
        path: path.map(|p| p.display().to_string()),
        status: status.into(),
        command,
        note: note.map(str::to_owned),
    });
}

fn backup_name(path: &Path) -> PathBuf {
    PathBuf::from(format!(
        "{}.bak-{}",
        path.display(),
        chrono::Utc::now().format("%Y%m%dT%H%M%S%.9fZ")
    ))
}

fn backup_file(path: &Path) -> Result<()> {
    use std::io::Write;
    let data = fs::read(path)?;
    let dst = backup_name(path);
    let permissions = fs::metadata(path)?.permissions();
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        use std::os::unix::fs::PermissionsExt;
        options.mode(permissions.mode() & 0o777);
    }
    let mut file = options.open(&dst)?;
    file.write_all(&data)?;
    fs::set_permissions(dst, permissions)?;
    Ok(())
}

fn save_state(path: &Path, state: &BTreeMap<String, Entry>) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(state)?;
    bytes.push(b'\n');
    if path.exists() {
        if fs::read(path)? == bytes {
            return Ok(());
        }
        backup_file(path)?;
    }
    private_parent(path)?;
    write_private(path, &bytes)
}

fn private_parent(path: &Path) -> Result<()> {
    let parent = path.parent().context("path has no parent")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true).mode(0o700).create(parent)?;
    }
    #[cfg(not(unix))]
    fs::create_dir_all(parent)?;
    Ok(())
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    Ok(())
}

pub fn materialize_skill(path: &Path) -> Result<()> {
    if path.exists() {
        if !skill_matches(path)? {
            bail!("embedded skill cache at {} was modified", path.display());
        }
        return Ok(());
    }
    install_skill(path)
}

fn install_skill(path: &Path) -> Result<()> {
    for (rel, text) in crate::skill::files() {
        let target = path.join(rel);
        private_parent(&target)?;
        write_private(&target, text.as_bytes())?;
    }
    Ok(())
}

fn skill_matches(path: &Path) -> Result<bool> {
    if !path.is_dir() {
        return Ok(false);
    }
    let mut expected = std::collections::BTreeSet::new();
    for (rel, text) in crate::skill::files() {
        expected.insert(PathBuf::from(rel));
        let target = path.join(rel);
        if !fs::symlink_metadata(&target).is_ok_and(|m| m.is_file()) {
            return Ok(false);
        }
        if fs::read(target)? != text.as_bytes() {
            return Ok(false);
        }
    }
    fn visit(dir: &Path, base: &Path, expected: &std::collections::BTreeSet<PathBuf>) -> Result<bool> {
        for item in fs::read_dir(dir)? {
            let entry = item?;
            let p = entry.path();
            if entry.file_type()?.is_dir() {
                if !visit(&p, base, expected)? {
                    return Ok(false);
                }
            } else if !expected.contains(&p.strip_prefix(base)?.to_path_buf()) {
                return Ok(false);
            }
        }
        Ok(true)
    }
    visit(path, path, &expected)
}

fn skill_action(report: &mut Report, agent: &str, path: &Path, rec: &mut Entry, opts: &Options) -> Result<bool> {
    let exists = fs::symlink_metadata(path).is_ok();
    if opts.uninstall {
        if !rec.skill_installed {
            action(report, agent, "skill", Some(path), "not_managed", None, None);
            return Ok(false);
        }
        if exists && !skill_matches(path)? {
            action(report, agent, "skill", Some(path), "modified_skip", None, None);
            return Ok(false);
        }
        if opts.dry_run {
            action(
                report,
                agent,
                "skill",
                Some(path),
                if exists { "would_remove" } else { "would_restore" },
                None,
                None,
            );
            return Ok(false);
        }
        if exists {
            fs::remove_dir_all(path)?;
        }
        if !rec.skill_backup.is_empty() {
            fs::rename(&rec.skill_backup, path)?;
        }
        rec.skill_installed = false;
        rec.skill_backup.clear();
        action(report, agent, "skill", Some(path), "removed", None, None);
        return Ok(true);
    }
    if exists {
        if skill_matches(path)? {
            action(report, agent, "skill", Some(path), "unchanged", None, None);
            return Ok(false);
        }
        if rec.skill_installed {
            action(report, agent, "skill", Some(path), "modified_skip", None, None);
            return Ok(false);
        }
    }
    if opts.dry_run {
        action(
            report,
            agent,
            "skill",
            Some(path),
            "would_install",
            exists.then_some("existing skill will be backed up"),
            None,
        );
        return Ok(false);
    }
    if exists {
        let backup = backup_name(path);
        fs::rename(path, &backup)?;
        rec.skill_backup = backup.display().to_string();
    }
    if let Err(error) = install_skill(path) {
        let _ = fs::remove_dir_all(path);
        if !rec.skill_backup.is_empty() {
            let _ = fs::rename(&rec.skill_backup, path);
            rec.skill_backup.clear();
        }
        return Err(error);
    }
    rec.skill_installed = true;
    action(report, agent, "skill", Some(path), "installed", None, None);
    Ok(true)
}

fn mcp_action(report: &mut Report, agent: &str, home: &Path, rec: &mut Entry, opts: &Options) -> Result<bool> {
    if agent == "hermes" {
        action(
            report,
            agent,
            "mcp",
            None,
            "unsupported",
            Some("skill only; MCP config path is not established"),
            None,
        );
        return Ok(false);
    }
    let bin = opts.mcp_binary.clone().or_else(mcp_binary);
    if !opts.uninstall && !bin.as_deref().is_some_and(executable) {
        action(
            report,
            agent,
            "mcp",
            bin.as_deref(),
            "missing_binary",
            Some("build or install mixlayer-mcp beside mixlayer"),
            None,
        );
        return Ok(false);
    }
    let bin = bin.unwrap_or_default();
    match agent {
        "codex" => toml_mcp(report, agent, &home.join(".codex/config.toml"), &bin, rec, opts),
        "cursor" => json_mcp(report, agent, &home.join(".cursor/mcp.json"), &bin, rec, opts),
        "claude-desktop" => {
            let base = if cfg!(target_os = "macos") {
                home.join("Library/Application Support/Claude")
            } else if cfg!(windows) {
                std::env::var_os("APPDATA")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| home.to_owned())
                    .join("Claude")
            } else {
                home.join(".config/Claude")
            };
            json_mcp(report, agent, &base.join("claude_desktop_config.json"), &bin, rec, opts)
        }
        "claude-code" => claude_mcp(report, home, &bin, rec, opts),
        _ => Ok(false),
    }
}

fn read_json_object(path: &Path) -> Result<(Map<String, Value>, bool)> {
    if !path.exists() {
        return Ok((Map::new(), true));
    }
    let doc: Value = serde_json::from_slice(&fs::read(path)?)?;
    match doc {
        Value::Object(map) => Ok((map, false)),
        Value::Null => Ok((Map::new(), false)),
        _ => bail!("{}: MCP configuration must be a JSON object", path.display()),
    }
}

fn write_config(path: &Path, bytes: &[u8]) -> Result<()> {
    private_parent(path)?;
    if path.exists() {
        backup_file(path)?;
    }
    write_private(path, bytes)
}

fn json_mcp(
    report: &mut Report,
    agent: &str,
    path: &Path,
    bin: &Path,
    rec: &mut Entry,
    opts: &Options,
) -> Result<bool> {
    let (mut doc, created) = read_json_object(path)?;
    let mut servers = match doc.remove("mcpServers") {
        Some(Value::Object(m)) => m,
        Some(Value::Null) | None => Map::new(),
        _ => bail!("{}: mcpServers must be an object", path.display()),
    };
    let current = servers.get("mixlayer").cloned();
    if opts.uninstall {
        if !rec.mcp_installed {
            action(report, agent, "mcp", Some(path), "not_managed", None, None);
            return Ok(false);
        }
        if current != rec.mcp_value {
            action(report, agent, "mcp", Some(path), "modified_skip", None, None);
            return Ok(false);
        }
        if opts.dry_run {
            action(report, agent, "mcp", Some(path), "would_remove", None, None);
            return Ok(false);
        }
        servers.remove("mixlayer");
        if !servers.is_empty() {
            doc.insert("mcpServers".into(), Value::Object(servers));
        }
        if rec.mcp_file_created && doc.is_empty() {
            backup_file(path)?;
            fs::remove_file(path)?;
        } else {
            let mut bytes = serde_json::to_vec_pretty(&doc)?;
            bytes.push(b'\n');
            write_config(path, &bytes)?;
        }
        rec.mcp_installed = false;
        rec.mcp_file_created = false;
        rec.mcp_value = None;
        action(report, agent, "mcp", Some(path), "removed", None, None);
        return Ok(true);
    }
    let value = json!({"command": bin.display().to_string(), "args": []});
    if let Some(current) = current {
        action(
            report,
            agent,
            "mcp",
            Some(path),
            if current == value { "unchanged" } else { "conflict_skip" },
            None,
            None,
        );
        return Ok(false);
    }
    if opts.dry_run {
        action(report, agent, "mcp", Some(path), "would_register", None, None);
        return Ok(false);
    }
    servers.insert("mixlayer".into(), value.clone());
    doc.insert("mcpServers".into(), Value::Object(servers));
    let mut bytes = serde_json::to_vec_pretty(&doc)?;
    bytes.push(b'\n');
    write_config(path, &bytes)?;
    rec.mcp_installed = true;
    rec.mcp_file_created = created;
    rec.mcp_value = Some(value);
    action(report, agent, "mcp", Some(path), "registered", None, None);
    Ok(true)
}

fn toml_mcp(
    report: &mut Report,
    agent: &str,
    path: &Path,
    bin: &Path,
    rec: &mut Entry,
    opts: &Options,
) -> Result<bool> {
    let created = !path.exists();
    let bytes = if created { Vec::new() } else { fs::read(path)? };
    let doc: toml::Value = if bytes.is_empty() {
        toml::Value::Table(Default::default())
    } else {
        toml::from_str(std::str::from_utf8(&bytes)?)?
    };
    let servers = doc.get("mcp_servers");
    if servers.is_some_and(|v| !v.is_table()) {
        bail!("{}: mcp_servers must be a table", path.display());
    }
    let exists = servers.and_then(|v| v.get("mixlayer")).is_some();
    if opts.uninstall {
        if !rec.mcp_installed {
            action(report, agent, "mcp", Some(path), "not_managed", None, None);
            return Ok(false);
        }
        let raw = String::from_utf8(bytes)?;
        if rec.mcp_snippet.is_empty() || !raw.contains(&rec.mcp_snippet) {
            action(report, agent, "mcp", Some(path), "modified_skip", None, None);
            return Ok(false);
        }
        if opts.dry_run {
            action(report, agent, "mcp", Some(path), "would_remove", None, None);
            return Ok(false);
        }
        let updated = raw.replacen(&rec.mcp_snippet, "", 1);
        if rec.mcp_file_created && updated.trim().is_empty() {
            backup_file(path)?;
            fs::remove_file(path)?;
        } else {
            write_config(path, updated.as_bytes())?;
        }
        rec.mcp_installed = false;
        rec.mcp_file_created = false;
        rec.mcp_snippet.clear();
        action(report, agent, "mcp", Some(path), "removed", None, None);
        return Ok(true);
    }
    if exists {
        action(
            report,
            agent,
            "mcp",
            Some(path),
            if rec.mcp_installed {
                "unchanged"
            } else {
                "conflict_skip"
            },
            None,
            None,
        );
        return Ok(false);
    }
    if opts.dry_run {
        action(report, agent, "mcp", Some(path), "would_register", None, None);
        return Ok(false);
    }
    let prefix = if bytes.is_empty() {
        ""
    } else if bytes.last() == Some(&b'\n') {
        "\n"
    } else {
        "\n\n"
    };
    let encoded = serde_json::to_string(&bin.display().to_string())?;
    let snippet = format!(
        "{prefix}# mixlayer agent setup begin\n[mcp_servers.mixlayer]\ncommand = {encoded}\nargs = []\n# mixlayer agent setup end\n"
    );
    let mut updated = bytes;
    updated.extend_from_slice(snippet.as_bytes());
    write_config(path, &updated)?;
    rec.mcp_installed = true;
    rec.mcp_file_created = created;
    rec.mcp_snippet = snippet;
    action(report, agent, "mcp", Some(path), "registered", None, None);
    Ok(true)
}

fn hash_json(v: &Value) -> String {
    hex::encode(Sha256::digest(serde_json::to_vec(v).unwrap_or_default()))
}

fn claude_mcp(report: &mut Report, home: &Path, bin: &Path, rec: &mut Entry, opts: &Options) -> Result<bool> {
    let path = std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.to_owned())
        .join(".claude.json");
    let claude = find_executable(if cfg!(windows) { "claude.exe" } else { "claude" });
    if claude.is_none() {
        let cmd = if opts.uninstall {
            "claude mcp remove --scope user mixlayer".into()
        } else {
            format!(
                "claude mcp add --scope user mixlayer -- {:?}",
                bin.display().to_string()
            )
        };
        action(
            report,
            "claude-code",
            "mcp",
            Some(&path),
            "manual_command",
            None,
            Some(cmd),
        );
        return Ok(false);
    }
    if opts.uninstall && !rec.mcp_installed {
        action(report, "claude-code", "mcp", Some(&path), "not_managed", None, None);
        return Ok(false);
    }
    let current = read_claude_entry(&path)?;
    if !opts.uninstall {
        if rec.mcp_installed {
            let status = if current.as_ref().is_some_and(|v| hash_json(v) == rec.mcp_hash) {
                "unchanged"
            } else {
                "modified_skip"
            };
            action(report, "claude-code", "mcp", Some(&path), status, None, None);
            return Ok(false);
        }
        if current.is_some() {
            action(report, "claude-code", "mcp", Some(&path), "conflict_skip", None, None);
            return Ok(false);
        }
    } else {
        if current.is_none() {
            if opts.dry_run {
                action(report, "claude-code", "mcp", Some(&path), "would_remove", None, None);
                return Ok(false);
            }
            rec.mcp_installed = false;
            rec.mcp_hash.clear();
            action(report, "claude-code", "mcp", Some(&path), "removed", None, None);
            return Ok(true);
        }
        if current.as_ref().is_some_and(|v| hash_json(v) != rec.mcp_hash) {
            action(report, "claude-code", "mcp", Some(&path), "modified_skip", None, None);
            return Ok(false);
        }
    }
    let args: Vec<String> = if opts.uninstall {
        vec!["mcp", "remove", "--scope", "user", "mixlayer"]
            .into_iter()
            .map(str::to_owned)
            .collect()
    } else {
        ["mcp", "add", "--scope", "user", "mixlayer", "--"]
            .into_iter()
            .map(str::to_owned)
            .chain(std::iter::once(bin.display().to_string()))
            .collect()
    };
    let command = format!("claude {}", args.join(" "));
    if opts.dry_run {
        action(
            report,
            "claude-code",
            "mcp",
            Some(&path),
            if opts.uninstall {
                "would_remove"
            } else {
                "would_register"
            },
            None,
            Some(command),
        );
        return Ok(false);
    }
    if path.exists() {
        backup_file(&path)?;
    }
    let output = Command::new(claude.unwrap()).args(&args).output()?;
    if !output.status.success() {
        bail!(
            "claude mcp command failed: {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    if opts.uninstall {
        rec.mcp_installed = false;
        rec.mcp_hash.clear();
    } else {
        let value = read_claude_entry(&path)?.context("claude mcp add succeeded but config has no mixlayer entry")?;
        rec.mcp_installed = true;
        rec.mcp_hash = hash_json(&value);
    }
    action(
        report,
        "claude-code",
        "mcp",
        Some(&path),
        if opts.uninstall { "removed" } else { "registered" },
        None,
        Some(command),
    );
    Ok(true)
}

fn read_claude_entry(path: &Path) -> Result<Option<Value>> {
    if !path.exists() {
        return Ok(None);
    }
    let doc: Value = serde_json::from_slice(&fs::read(path)?).context("reading Claude MCP metadata")?;
    Ok(doc.get("mcpServers").and_then(|v| v.get("mixlayer")).cloned())
}
