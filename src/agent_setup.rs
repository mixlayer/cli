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
    /// Also register (or, with `uninstall`, remove) the hosted Platform MCP server.
    pub platform: Option<Platform>,
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
    /// `skill_tree_hash` of the copy setup last wrote, so a later setup can
    /// tell an older Mixlayer skill (safe to update) from one the user edited.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    skill_hash: String,
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
    if let Some(p) = opts.platform.as_ref().filter(|_| !opts.uninstall) {
        check_platform_key(&mut report, p);
    }
    // Uninstall removes managed Platform entries even without --platform; the
    // stored record, not this value, decides what is removed.
    let platform = opts.platform.clone().or_else(|| {
        opts.uninstall.then(|| Platform {
            url: "https://api.mixlayer.com/v1/mcp".into(),
            key_env: "MIXLAYER_API_KEY".into(),
        })
    });
    for agent in targets {
        let platform_key = format!("{agent}:{PLATFORM_KIND}");
        let mut steps = vec![agent.to_owned()];
        if platform.is_some() && (opts.platform.is_some() || (opts.uninstall && state.contains_key(&platform_key))) {
            steps.push(platform_key.clone());
        }
        // Undo in reverse install order, so the entry that created a shared
        // config file is removed last and can delete the file.
        if opts.uninstall {
            steps.reverse();
        }
        for key in steps {
            let mut rec = state.get(&key).cloned().unwrap_or_default();
            let previous = rec.clone();
            let result = (|| -> Result<()> {
                if key == platform_key {
                    let platform = platform.as_ref().expect("platform step implies a platform");
                    changed |= platform_action(&mut report, agent, &home, &mut rec, &opts, platform)?;
                    return Ok(());
                }
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
                    state.remove(&key);
                } else {
                    state.insert(key.clone(), rec);
                }
            }
            if let Err(error) = result {
                if !opts.dry_run && (changed || state.get(&key) != Some(&previous)) {
                    save_state(&state_path, &state)?;
                }
                return Err(error);
            }
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

/// Skill trees shipped before setup recorded `skill_hash`, as computed by
/// `skill_tree_hash`. An install from one of those versions is updated like
/// any other unmodified install. Frozen: newer versions are tracked through
/// the recorded hash, so this list never needs to grow.
const LEGACY_SKILL_HASHES: &[&str] = &[
    "3f89af32ad4fab70c84af5891c4ffc01c16d38215243f194df4d043c98359a7c",
    "9e1fb25ea91c8b5397a62e05438b15e9ce23d0a7f9c194a5ec7a60ed877fa473",
    "f2d2fbcc9c097023f01565e43b43f89b77dd2cae557a37ff792d594c98cca5c6",
    "1d3e4996a5bd7fc5420e852aadfecf55069a3cb4ed258b4831c57415e529a027", // v0.1.0
];

/// SHA-256 over every file in a skill directory: sorted `/`-separated relative
/// paths, each followed by NUL, the bytes and NUL. `None` if the tree holds
/// anything other than regular files and directories.
fn skill_tree_hash(path: &Path) -> Result<Option<String>> {
    fn visit(dir: &Path, base: &Path, out: &mut Vec<(String, Vec<u8>)>) -> Result<bool> {
        for item in fs::read_dir(dir)? {
            let entry = item?;
            let p = entry.path();
            let kind = entry.file_type()?;
            if kind.is_dir() {
                if !visit(&p, base, out)? {
                    return Ok(false);
                }
            } else if kind.is_file() {
                let rel = p
                    .strip_prefix(base)?
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy())
                    .collect::<Vec<_>>()
                    .join("/");
                out.push((rel, fs::read(&p)?));
            } else {
                return Ok(false);
            }
        }
        Ok(true)
    }
    if !fs::symlink_metadata(path).is_ok_and(|m| m.is_dir()) {
        return Ok(None);
    }
    let mut files = Vec::new();
    if !visit(path, path, &mut files)? {
        return Ok(None);
    }
    files.sort();
    let mut h = Sha256::new();
    for (rel, data) in files {
        h.update(rel.as_bytes());
        h.update([0]);
        h.update(&data);
        h.update([0]);
    }
    Ok(Some(hex::encode(h.finalize())))
}

/// Hash of the skill this binary embeds, as `install_skill` writes it.
fn embedded_skill_hash() -> String {
    let mut files: Vec<(&str, &str)> = crate::skill::files().to_vec();
    files.sort();
    let mut h = Sha256::new();
    for (rel, text) in files {
        h.update(rel.as_bytes());
        h.update([0]);
        h.update(text.as_bytes());
        h.update([0]);
    }
    hex::encode(h.finalize())
}

/// True when a managed skill on disk is exactly what some Mixlayer version
/// installed: the hash setup recorded, or a pre-tracking release.
fn skill_is_pristine(path: &Path, rec: &Entry) -> Result<bool> {
    let Some(hash) = skill_tree_hash(path)? else {
        return Ok(false);
    };
    Ok(if rec.skill_hash.is_empty() {
        LEGACY_SKILL_HASHES.contains(&hash.as_str())
    } else {
        hash == rec.skill_hash
    })
}

/// State of each skill `agent setup` manages, for `doctor`: `current`,
/// `outdated` (an older Mixlayer version; rerun setup), `modified` (edited by
/// the user; setup leaves it alone) or `deleted`. Empty when setup never ran.
/// (Not "missing": `doctor --fail-on` treats that word as an error.)
pub fn managed_skill_report() -> Vec<Value> {
    let Ok(home) = home() else {
        return Vec::new();
    };
    let state_path = home.join(".config/mixlayer/agent-setup.json");
    let Ok(bytes) = fs::read(&state_path) else {
        return Vec::new();
    };
    let Ok(state) = serde_json::from_slice::<BTreeMap<String, Entry>>(&bytes) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for agent in ORDER {
        let Some(rec) = state.get(*agent).filter(|r| r.skill_installed) else {
            continue;
        };
        let Some(rel) = skill_location(agent) else {
            continue;
        };
        let path = rel.split('/').fold(home.clone(), |p, part| p.join(part));
        let status = if !fs::symlink_metadata(&path).is_ok() {
            "deleted"
        } else if skill_matches(&path).unwrap_or(false) {
            "current"
        } else if skill_is_pristine(&path, rec).unwrap_or(false) {
            "outdated"
        } else {
            "modified"
        };
        out.push(json!({"agent": agent, "path": path.display().to_string(), "status": status}));
    }
    out
}

fn skill_action(report: &mut Report, agent: &str, path: &Path, rec: &mut Entry, opts: &Options) -> Result<bool> {
    let exists = fs::symlink_metadata(path).is_ok();
    if opts.uninstall {
        if !rec.skill_installed {
            action(report, agent, "skill", Some(path), "not_managed", None, None);
            return Ok(false);
        }
        if exists && !skill_matches(path)? && !skill_is_pristine(path, rec)? {
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
        rec.skill_hash.clear();
        action(report, agent, "skill", Some(path), "removed", None, None);
        return Ok(true);
    }
    if exists {
        if skill_matches(path)? {
            action(report, agent, "skill", Some(path), "unchanged", None, None);
            // Record the hash for installs made before setup tracked it, so
            // the next upgrade can update this copy.
            if rec.skill_installed && rec.skill_hash.is_empty() && !opts.dry_run {
                rec.skill_hash = embedded_skill_hash();
                return Ok(true);
            }
            return Ok(false);
        }
        if rec.skill_installed {
            if !skill_is_pristine(path, rec)? {
                action(
                    report,
                    agent,
                    "skill",
                    Some(path),
                    "modified_skip",
                    Some(
                        "the installed skill was edited; remove it or merge the new version by hand (see `mixlayer agent skill path`)",
                    ),
                    None,
                );
                return Ok(false);
            }
            if opts.dry_run {
                action(report, agent, "skill", Some(path), "would_update", None, None);
                return Ok(false);
            }
            // An older, unedited Mixlayer skill: replace it with this version.
            let old = PathBuf::from(format!("{}.updating", path.display()));
            if fs::symlink_metadata(&old).is_ok() {
                fs::remove_dir_all(&old)?;
            }
            fs::rename(path, &old)?;
            if let Err(error) = install_skill(path) {
                let _ = fs::remove_dir_all(path);
                let _ = fs::rename(&old, path);
                return Err(error);
            }
            fs::remove_dir_all(&old)?;
            rec.skill_hash = embedded_skill_hash();
            action(report, agent, "skill", Some(path), "updated", None, None);
            return Ok(true);
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
    rec.skill_hash = embedded_skill_hash();
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
    let server = Server::local(&bin);
    match agent {
        "codex" => toml_mcp(report, agent, &home.join(".codex/config.toml"), &server, rec, opts),
        "cursor" => json_mcp(report, agent, &home.join(".cursor/mcp.json"), &server, rec, opts),
        "claude-desktop" => json_mcp(report, agent, &claude_desktop_config(home), &server, rec, opts),
        "claude-code" => claude_mcp(report, home, &server, rec, opts),
        _ => Ok(false),
    }
}

fn claude_desktop_config(home: &Path) -> PathBuf {
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
    base.join("claude_desktop_config.json")
}

/// The hosted Platform MCP server: organizations, members, API keys, usage
/// and billing as tools, authorized by the key in `key_env`. The agent config
/// holds only the variable's name; the agent reads the value at launch.
fn platform_action(
    report: &mut Report,
    agent: &str,
    home: &Path,
    rec: &mut Entry,
    opts: &Options,
    platform: &Platform,
) -> Result<bool> {
    let unsupported = |report: &mut Report, note: &str| {
        if !opts.uninstall {
            action(report, agent, PLATFORM_KIND, None, "unsupported", Some(note), None);
        }
        Ok(false)
    };
    match agent {
        "codex" => toml_mcp(
            report,
            agent,
            &home.join(".codex/config.toml"),
            &Server::platform(platform, agent),
            rec,
            opts,
        ),
        "cursor" => json_mcp(
            report,
            agent,
            &home.join(".cursor/mcp.json"),
            &Server::platform(platform, agent),
            rec,
            opts,
        ),
        "claude-code" => claude_mcp(report, home, &Server::platform(platform, agent), rec, opts),
        "claude-desktop" => unsupported(
            report,
            "Claude Desktop config cannot read the key from an environment variable; use Claude Code, Codex or Cursor",
        ),
        _ => unsupported(report, "MCP config path is not established"),
    }
}

const PLATFORM_KIND: &str = "platform-mcp";
const PLATFORM_NAME: &str = "mixlayer-platform";

/// Where the hosted Platform MCP server lives and which variable holds its key.
#[derive(Clone, Debug)]
pub struct Platform {
    /// Full MCP endpoint, e.g. `https://api.mixlayer.com/v1/mcp`.
    pub url: String,
    /// Environment variable the agent reads for the bearer token.
    pub key_env: String,
}

impl Platform {
    pub fn new(base: &str, key_env: &str) -> Result<Self> {
        let valid = !key_env.is_empty()
            && !key_env.starts_with(|c: char| c.is_ascii_digit())
            && key_env.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
        if !valid {
            bail!("--key-env must be an environment variable name (letters, digits and _), got {key_env:?}");
        }
        Ok(Self {
            url: format!("{}/v1/mcp", base.trim_end_matches('/')),
            key_env: key_env.to_owned(),
        })
    }
}

/// One MCP server entry, in each agent's config format.
struct Server {
    name: &'static str,
    /// Action kind in the report: `mcp` or `platform-mcp`.
    kind: &'static str,
    /// `mcpServers` value for JSON configs (Cursor, Claude Desktop).
    json: Value,
    /// Body of the `[mcp_servers.<name>]` table for Codex.
    toml: String,
    /// Arguments to `claude` that register the server at user scope.
    claude_add: Vec<String>,
}

impl Server {
    fn local(bin: &Path) -> Self {
        let bin = bin.display().to_string();
        Self {
            name: "mixlayer",
            kind: "mcp",
            json: json!({"command": bin, "args": []}),
            toml: format!(
                "command = {}\nargs = []\n",
                serde_json::to_string(&bin).unwrap_or_default()
            ),
            claude_add: ["mcp", "add", "--scope", "user", "mixlayer", "--", &bin]
                .into_iter()
                .map(str::to_owned)
                .collect(),
        }
    }

    fn platform(p: &Platform, agent: &str) -> Self {
        // Each agent has its own syntax for "read this from the environment".
        let header = match agent {
            "cursor" => format!("Bearer ${{env:{}}}", p.key_env),
            _ => format!("Bearer ${{{}}}", p.key_env),
        };
        let quote = |s: &str| serde_json::to_string(s).unwrap_or_default();
        Self {
            name: PLATFORM_NAME,
            kind: PLATFORM_KIND,
            json: json!({"url": p.url, "headers": {"Authorization": header}}),
            toml: format!(
                "url = {}\nbearer_token_env_var = {}\n",
                quote(&p.url),
                quote(&p.key_env)
            ),
            claude_add: [
                "mcp",
                "add",
                "--transport",
                "http",
                "--scope",
                "user",
                PLATFORM_NAME,
                &p.url,
                "--header",
                &format!("Authorization: {header}"),
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        }
    }

    /// The `claude` arguments as a command a user can paste into a POSIX shell.
    fn claude_add_display(&self) -> String {
        self.claude_add
            .iter()
            .map(|a| shell_quote(a))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

fn shell_quote(arg: &str) -> String {
    if !arg.is_empty()
        && arg
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./:=@%+,".contains(c))
    {
        return arg.to_owned();
    }
    format!("'{}'", arg.replace('\'', r"'\''"))
}

/// Read-only check that the key the agent will use can call the Platform API.
/// Never fails setup: the result is reported as a `platform-key` action.
fn check_platform_key(report: &mut Report, p: &Platform) {
    let mut push = |status: &str, note: String| {
        action(report, "platform", "platform-key", None, status, Some(&note), None);
    };
    let keys_page = crate::api::console_url("/app/api-keys", "agent-setup");
    let Some(key) = std::env::var(&p.key_env).ok().filter(|v| !v.trim().is_empty()) else {
        push(
            "missing",
            format!(
                "{} is not set here. Set it in the environment your agent starts from; the agent reads it at launch",
                p.key_env
            ),
        );
        return;
    };
    let orgs_url = format!("{}/v1/organizations", p.url.trim_end_matches("/v1/mcp"));
    let response = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .and_then(|c| c.get(&orgs_url).bearer_auth(key.trim()).send());
    match response.map(|r| r.status().as_u16()) {
        Ok(200) => push("ok", format!("{} can read the Platform API", p.key_env)),
        Ok(403) => push(
            "no_platform_permission",
            format!(
                "the Platform API refused {} (HTTP 403). Give the key api-read (or api-write/api-admin to let the agent change things) at {keys_page}",
                p.key_env
            ),
        ),
        Ok(401) => push(
            "rejected",
            format!(
                "the Platform API rejected {} (HTTP 401); check the key at {keys_page}",
                p.key_env
            ),
        ),
        Ok(status) => push("error", format!("unexpected HTTP {status} from {orgs_url}")),
        Err(_) => push("unreachable", format!("could not reach {orgs_url}")),
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
    server: &Server,
    rec: &mut Entry,
    opts: &Options,
) -> Result<bool> {
    let kind = server.kind;
    let (mut doc, created) = read_json_object(path)?;
    let mut servers = match doc.remove("mcpServers") {
        Some(Value::Object(m)) => m,
        Some(Value::Null) | None => Map::new(),
        _ => bail!("{}: mcpServers must be an object", path.display()),
    };
    let current = servers.get(server.name).cloned();
    if opts.uninstall {
        if !rec.mcp_installed {
            action(report, agent, kind, Some(path), "not_managed", None, None);
            return Ok(false);
        }
        if current != rec.mcp_value {
            action(report, agent, kind, Some(path), "modified_skip", None, None);
            return Ok(false);
        }
        if opts.dry_run {
            action(report, agent, kind, Some(path), "would_remove", None, None);
            return Ok(false);
        }
        servers.remove(server.name);
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
        action(report, agent, kind, Some(path), "removed", None, None);
        return Ok(true);
    }
    let value = server.json.clone();
    if let Some(current) = current {
        action(
            report,
            agent,
            kind,
            Some(path),
            if current == value { "unchanged" } else { "conflict_skip" },
            None,
            None,
        );
        return Ok(false);
    }
    if opts.dry_run {
        action(report, agent, kind, Some(path), "would_register", None, None);
        return Ok(false);
    }
    servers.insert(server.name.into(), value.clone());
    doc.insert("mcpServers".into(), Value::Object(servers));
    let mut bytes = serde_json::to_vec_pretty(&doc)?;
    bytes.push(b'\n');
    write_config(path, &bytes)?;
    rec.mcp_installed = true;
    rec.mcp_file_created = created;
    rec.mcp_value = Some(value);
    action(report, agent, kind, Some(path), "registered", None, None);
    Ok(true)
}

fn toml_mcp(
    report: &mut Report,
    agent: &str,
    path: &Path,
    server: &Server,
    rec: &mut Entry,
    opts: &Options,
) -> Result<bool> {
    let kind = server.kind;
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
    let exists = servers.and_then(|v| v.get(server.name)).is_some();
    if opts.uninstall {
        if !rec.mcp_installed {
            action(report, agent, kind, Some(path), "not_managed", None, None);
            return Ok(false);
        }
        let raw = String::from_utf8(bytes)?;
        if rec.mcp_snippet.is_empty() || !raw.contains(&rec.mcp_snippet) {
            action(report, agent, kind, Some(path), "modified_skip", None, None);
            return Ok(false);
        }
        if opts.dry_run {
            action(report, agent, kind, Some(path), "would_remove", None, None);
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
        action(report, agent, kind, Some(path), "removed", None, None);
        return Ok(true);
    }
    if exists {
        action(
            report,
            agent,
            kind,
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
        action(report, agent, kind, Some(path), "would_register", None, None);
        return Ok(false);
    }
    let prefix = if bytes.is_empty() {
        ""
    } else if bytes.last() == Some(&b'\n') {
        "\n"
    } else {
        "\n\n"
    };
    let snippet = format!(
        "{prefix}# mixlayer agent setup begin\n[mcp_servers.{}]\n{}# mixlayer agent setup end\n",
        server.name, server.toml
    );
    let mut updated = bytes;
    updated.extend_from_slice(snippet.as_bytes());
    write_config(path, &updated)?;
    rec.mcp_installed = true;
    rec.mcp_file_created = created;
    rec.mcp_snippet = snippet;
    action(report, agent, kind, Some(path), "registered", None, None);
    Ok(true)
}

fn hash_json(v: &Value) -> String {
    hex::encode(Sha256::digest(serde_json::to_vec(v).unwrap_or_default()))
}

fn claude_mcp(report: &mut Report, home: &Path, server: &Server, rec: &mut Entry, opts: &Options) -> Result<bool> {
    let kind = server.kind;
    let path = std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.to_owned())
        .join(".claude.json");
    let claude = find_executable(if cfg!(windows) { "claude.exe" } else { "claude" });
    if claude.is_none() {
        let cmd = if opts.uninstall {
            format!("claude mcp remove --scope user {}", server.name)
        } else {
            format!("claude {}", server.claude_add_display())
        };
        action(
            report,
            "claude-code",
            kind,
            Some(&path),
            "manual_command",
            None,
            Some(cmd),
        );
        return Ok(false);
    }
    if opts.uninstall && !rec.mcp_installed {
        action(report, "claude-code", kind, Some(&path), "not_managed", None, None);
        return Ok(false);
    }
    let current = read_claude_entry(&path, server.name)?;
    if !opts.uninstall {
        if rec.mcp_installed {
            let status = if current.as_ref().is_some_and(|v| hash_json(v) == rec.mcp_hash) {
                "unchanged"
            } else {
                "modified_skip"
            };
            action(report, "claude-code", kind, Some(&path), status, None, None);
            return Ok(false);
        }
        if current.is_some() {
            action(report, "claude-code", kind, Some(&path), "conflict_skip", None, None);
            return Ok(false);
        }
    } else {
        if current.is_none() {
            if opts.dry_run {
                action(report, "claude-code", kind, Some(&path), "would_remove", None, None);
                return Ok(false);
            }
            rec.mcp_installed = false;
            rec.mcp_hash.clear();
            action(report, "claude-code", kind, Some(&path), "removed", None, None);
            return Ok(true);
        }
        if current.as_ref().is_some_and(|v| hash_json(v) != rec.mcp_hash) {
            action(report, "claude-code", kind, Some(&path), "modified_skip", None, None);
            return Ok(false);
        }
    }
    let args: Vec<String> = if opts.uninstall {
        ["mcp", "remove", "--scope", "user", server.name]
            .into_iter()
            .map(str::to_owned)
            .collect()
    } else {
        server.claude_add.clone()
    };
    let command = if opts.uninstall {
        format!("claude {}", args.join(" "))
    } else {
        format!("claude {}", server.claude_add_display())
    };
    if opts.dry_run {
        action(
            report,
            "claude-code",
            kind,
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
        let value = read_claude_entry(&path, server.name)?
            .with_context(|| format!("claude mcp add succeeded but config has no {} entry", server.name))?;
        rec.mcp_installed = true;
        rec.mcp_hash = hash_json(&value);
    }
    action(
        report,
        "claude-code",
        kind,
        Some(&path),
        if opts.uninstall { "removed" } else { "registered" },
        None,
        Some(command),
    );
    Ok(true)
}

fn read_claude_entry(path: &Path, name: &str) -> Result<Option<Value>> {
    if !path.exists() {
        return Ok(None);
    }
    let doc: Value = serde_json::from_slice(&fs::read(path)?).context("reading Claude MCP metadata")?;
    Ok(doc.get("mcpServers").and_then(|v| v.get(name)).cloned())
}

/// Human summary of a setup report, printed when stdout is a terminal and no
/// machine output was asked for. Pipes, `--json` and agents still get JSON.
pub fn render_human(report: &Report, home: Option<&Path>, color: bool) -> String {
    let paint = |code: &str, s: &str| {
        if color {
            format!("\x1b[{code}m{s}\x1b[0m")
        } else {
            s.to_string()
        }
    };
    let tidy = |p: &str| match home.and_then(|h| Path::new(p).strip_prefix(h).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Some(rest) => format!("~{}{}", if p.contains('\\') { '\\' } else { '/' }, rest.display()),
        None => p.to_string(),
    };
    let mut out = String::new();
    if report.dry_run {
        let verb = if report.uninstall { "remove" } else { "change" };
        out.push_str(&paint(
            "2",
            &format!("Preview only: this is what setup would {verb}. Nothing was written."),
        ));
        out.push_str("\n\n");
    }
    let mut last_agent = "";
    let mut warnings = 0;
    for a in &report.actions {
        if a.agent != last_agent {
            if !last_agent.is_empty() {
                out.push('\n');
            }
            out.push_str(&paint("1", agent_label(&a.agent)));
            out.push('\n');
            last_agent = &a.agent;
        }
        let (mark, code) = match status_tone(&a.status) {
            Tone::Done => ("✓", "32"),
            Tone::Same => ("✓", "2"),
            Tone::Pending => ("→", "36"),
            Tone::Warn => {
                warnings += 1;
                ("!", "33")
            }
        };
        let what = format!("{} {}", kind_label(&a.kind), status_label(&a.status));
        out.push_str(&format!("  {} {what}", paint(code, mark)));
        if let Some(p) = &a.path {
            out.push_str(&format!("  {}", paint("2", &tidy(p))));
        }
        out.push('\n');
        if let Some(note) = &a.note {
            out.push_str(&format!("    {note}\n"));
        }
        if let Some(cmd) = a
            .command
            .as_ref()
            .filter(|_| matches!(status_tone(&a.status), Tone::Warn))
        {
            out.push_str(&format!("    run: {cmd}\n"));
        }
    }
    out.push('\n');
    if report.dry_run {
        out.push_str("Run again with --yes (and without --dry-run) to apply.\n");
    } else if warnings > 0 {
        out.push_str(&format!("Done, with {warnings} item(s) to check above.\n"));
    } else if report.uninstall {
        out.push_str("Done. Mixlayer is removed from these agents.\n");
    } else {
        out.push_str("Done. Restart your agent to load Mixlayer.\n");
    }
    out
}

enum Tone {
    Done,
    Same,
    Pending,
    Warn,
}

fn status_tone(status: &str) -> Tone {
    match status {
        "installed" | "registered" | "updated" | "removed" | "deleted" | "restored" => Tone::Done,
        "unchanged" | "current" => Tone::Same,
        s if s.starts_with("would_") => Tone::Pending,
        _ => Tone::Warn,
    }
}

fn status_label(status: &str) -> String {
    match status {
        "unchanged" | "current" => "already set up".into(),
        "manual_command" => "needs one manual step".into(),
        "modified_skip" => "left alone (you edited it)".into(),
        "conflict_skip" => "left alone (another server uses that name)".into(),
        "not_managed" => "left alone (not added by Mixlayer)".into(),
        "missing_binary" => "can't find mixlayer-mcp".into(),
        "no_platform_permission" => "key lacks platform permissions".into(),
        "unsupported" => "not supported here".into(),
        "outdated" => "out of date".into(),
        "rejected" => "rejected by the API".into(),
        "error" => "failed".into(),
        s => match s.strip_prefix("would_") {
            Some(verb) => format!("will be {}", past_tense(verb)),
            None => s.replace('_', " "),
        },
    }
}

fn past_tense(verb: &str) -> String {
    match verb {
        "install" => "installed".into(),
        "register" => "registered".into(),
        "update" => "updated".into(),
        "remove" => "removed".into(),
        "restore" => "restored".into(),
        v => format!("{v}ed"),
    }
}

fn agent_label(agent: &str) -> &str {
    match agent {
        "claude-code" => "Claude Code",
        "claude-desktop" => "Claude Desktop",
        "codex" => "Codex",
        "cursor" => "Cursor",
        "hermes" => "Hermes",
        "platform" => "Mixlayer Platform",
        other => other,
    }
}

fn kind_label(kind: &str) -> &str {
    match kind {
        "skill" => "Skill",
        "mcp" => "MCP server",
        "platform-mcp" => "Platform MCP",
        "platform-key" => "API key",
        other => other,
    }
}

#[cfg(test)]
mod human_tests {
    use super::*;

    fn act(agent: &str, kind: &str, path: Option<&str>, status: &str, note: Option<&str>) -> Action {
        Action {
            agent: agent.into(),
            kind: kind.into(),
            path: path.map(Into::into),
            status: status.into(),
            command: Some("claude mcp add --scope user mixlayer -- /x/mixlayer-mcp".into()),
            note: note.map(Into::into),
        }
    }

    #[test]
    fn applied_setup_reads_as_a_checklist() {
        let report = Report {
            dry_run: false,
            uninstall: false,
            actions: vec![
                act(
                    "claude-code",
                    "skill",
                    Some("/home/u/.claude/skills/mixlayer"),
                    "installed",
                    None,
                ),
                act("claude-code", "mcp", Some("/home/u/.claude.json"), "registered", None),
            ],
        };
        let text = render_human(&report, Some(Path::new("/home/u")), false);
        assert_eq!(
            text,
            "Claude Code\n  ✓ Skill installed  ~/.claude/skills/mixlayer\n  ✓ MCP server registered  ~/.claude.json\n\nDone. Restart your agent to load Mixlayer.\n"
        );
        assert!(!text.contains('{'), "no JSON for people");
    }

    #[test]
    fn dry_run_says_nothing_changed_and_how_to_apply() {
        let report = Report {
            dry_run: true,
            uninstall: false,
            actions: vec![act(
                "codex",
                "skill",
                Some("/home/u/.codex/skills/mixlayer"),
                "would_install",
                None,
            )],
        };
        let text = render_human(&report, Some(Path::new("/home/u")), false);
        assert!(text.starts_with("Preview only: this is what setup would change. Nothing was written.\n\nCodex\n"));
        assert!(text.contains("  → Skill will be installed  ~/.codex/skills/mixlayer\n"));
        assert!(text.ends_with("Run again with --yes (and without --dry-run) to apply.\n"));
    }

    #[test]
    fn problems_show_their_note_and_manual_command() {
        let report = Report {
            dry_run: false,
            uninstall: false,
            actions: vec![
                act(
                    "platform",
                    "platform-key",
                    None,
                    "missing",
                    Some("MIXLAYER_API_KEY is not set here."),
                ),
                act("claude-code", "mcp", None, "missing_binary", None),
            ],
        };
        let text = render_human(&report, None, false);
        assert!(text.contains("Mixlayer Platform\n  ! API key missing\n    MIXLAYER_API_KEY is not set here.\n"));
        assert!(text.contains("  ! MCP server can't find mixlayer-mcp\n    run: claude mcp add"));
        assert!(text.ends_with("Done, with 2 item(s) to check above.\n"));
    }

    #[test]
    #[cfg(unix)]
    fn only_paths_inside_home_are_shortened() {
        let report = Report {
            dry_run: false,
            uninstall: false,
            actions: vec![
                act(
                    "cursor",
                    "skill",
                    Some("/home/u/.cursor/skills/mixlayer"),
                    "installed",
                    None,
                ),
                act(
                    "cursor",
                    "mcp",
                    Some("/home/user2/.cursor/mcp.json"),
                    "registered",
                    None,
                ),
            ],
        };
        let text = render_human(&report, Some(Path::new("/home/u")), false);
        assert!(text.contains("  ~/.cursor/skills/mixlayer\n"));
        assert!(text.contains("  /home/user2/.cursor/mcp.json\n"), "{text}");
    }

    #[test]
    fn color_is_only_added_when_asked() {
        let report = Report {
            dry_run: false,
            uninstall: false,
            actions: vec![act("cursor", "skill", None, "installed", None)],
        };
        assert!(!render_human(&report, None, false).contains('\x1b'));
        assert!(render_human(&report, None, true).contains("\x1b[32m✓\x1b[0m"));
    }
}
