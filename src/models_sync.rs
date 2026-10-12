// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

//! Register the public Mixlayer model feed in local agent harnesses.
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use toml_edit::{DocumentMut, Item, Table, value};

use crate::api::config;
use crate::exit::{self, Code};

const AGENTS: &[&str] = &["codex", "opencode", "hermes", "cursor", "claude-code", "claude-desktop"];

#[derive(Clone, Debug)]
pub struct Options {
    pub agent: String,
    pub default_model: Option<String>,
    pub dry_run: bool,
    pub uninstall: bool,
    pub force: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct Action {
    pub agent: String,
    pub status: String,
    pub path: String,
    pub added: Vec<String>,
    pub removed: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Report {
    pub dry_run: bool,
    pub uninstall: bool,
    pub actions: Vec<Action>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct Managed {
    installed: bool,
    hash: String,
    profile_hash: String,
    created: bool,
    profile_created: bool,
    ids: Vec<String>,
}

#[derive(Clone, Debug)]
struct Model {
    id: String,
    name: String,
    context: u64,
    output: u64,
    tools: bool,
    price: Option<f64>,
}

fn hash(text: &str) -> String {
    hex::encode(Sha256::digest(text.as_bytes()))
}
fn home() -> Result<PathBuf> {
    crate::paths::user_home().context("resolve user home")
}
fn codex_dir(home: &Path) -> PathBuf {
    std::env::var_os("CODEX_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".codex"))
}
fn opencode_dir(home: &Path) -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"))
        .join("opencode")
}
fn hermes_dir(home: &Path) -> PathBuf {
    std::env::var_os("HERMES_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".hermes"))
}
fn config_path(agent: &str, home: &Path) -> PathBuf {
    match agent {
        "codex" => codex_dir(home).join("config.toml"),
        "opencode" => opencode_dir(home).join("opencode.json"),
        "hermes" => hermes_dir(home).join("config.yaml"),
        "cursor" => home.join(".cursor"),
        "claude-code" => home.join(".claude"),
        "claude-desktop" => home.join(".config/Claude"),
        _ => home.to_path_buf(),
    }
}
fn detected(agent: &str, home: &Path) -> bool {
    let path = config_path(agent, home);
    match agent {
        "codex" | "opencode" | "hermes" => path.parent().is_some_and(Path::is_dir),
        _ => path.is_dir(),
    }
}
fn state_path(home: &Path) -> PathBuf {
    home.join(".config/mixlayer/agent-setup.json")
}
fn feed_path(home: &Path) -> PathBuf {
    home.join(".config/mixlayer/models-feed.json")
}
fn read_state(path: &Path) -> Result<Map<String, Value>> {
    if !path.exists() {
        return Ok(Map::new());
    }
    let v: Value = serde_json::from_slice(&fs::read(path)?)?;
    v.as_object().cloned().context("agent setup state must be an object")
}
fn managed(state: &Map<String, Value>, agent: &str) -> Managed {
    state
        .get(agent)
        .and_then(|v| v.get("models"))
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default()
}
fn save_state(path: &Path, state: &Map<String, Value>) -> Result<()> {
    write_file(path, &format!("{}\n", serde_json::to_string_pretty(state)?))
}
fn read_optional(path: &Path) -> Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(s) => Ok(Some(s)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}
fn write_file(path: &Path, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    if path.exists() {
        if fs::read(path)? == content.as_bytes() {
            return Ok(());
        }
        let backup = format!(
            "{}.bak-{}",
            path.display(),
            chrono::Utc::now().format("%Y%m%dT%H%M%S%.9fZ")
        );
        fs::copy(path, backup)?;
    }
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)?.write_all(content.as_bytes())?;
    Ok(())
}
fn set_managed(state: &mut Map<String, Value>, agent: &str, rec: &Managed) -> Result<()> {
    let entry = state.entry(agent.to_owned()).or_insert_with(|| json!({}));
    let map = entry
        .as_object_mut()
        .context("agent setup state entry must be an object")?;
    if rec.installed {
        map.insert("models".into(), serde_json::to_value(rec)?);
    } else {
        map.remove("models");
    }
    if map.is_empty() {
        state.remove(agent);
    }
    Ok(())
}
fn as_u64(v: &Value) -> Option<u64> {
    v.as_u64().or_else(|| v.as_str()?.parse().ok())
}
fn as_price(v: &Value) -> Option<f64> {
    v.as_f64().or_else(|| v.as_str()?.parse().ok())
}
fn parse_feed(raw: &Value) -> Result<Vec<Model>> {
    let data = raw
        .get("data")
        .and_then(Value::as_array)
        .context("model feed has no data array")?;
    let mut out = Vec::new();
    for item in data {
        let Some(id) = item.get("id").and_then(Value::as_str).filter(|s| !s.is_empty()) else {
            continue;
        };
        let pricing = &item["pricing"];
        let price = as_price(&pricing["completion"]).or_else(|| as_price(&pricing["output"]));
        out.push(Model {
            id: id.into(),
            name: item["name"].as_str().unwrap_or(id).into(),
            context: as_u64(&item["context_length"]).unwrap_or(131_072),
            output: as_u64(&item["max_output_length"]).unwrap_or(8_192),
            tools: item["supported_features"]
                .as_array()
                .is_some_and(|f| f.iter().any(|x| x == "tools")),
            price,
        });
    }
    if out.is_empty() {
        bail!("model feed contained no usable models");
    }
    Ok(out)
}
fn fetch_feed(base: &str) -> Result<Vec<Model>> {
    let origin = url::Url::parse(base)?.join("/_openrouter/models")?;
    let response = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?
        .get(origin)
        .send()?
        .error_for_status()?;
    parse_feed(&response.json::<Value>()?)
}
fn key_catalog() -> Result<Vec<Model>> {
    let mut root = crate::cli::build();
    let matches = root.try_get_matches_from_mut(["mxl", "models", "list"])?;
    let leaf = matches
        .subcommand()
        .and_then(|(_, m)| m.subcommand())
        .map(|(_, m)| m)
        .context("models list matches")?;
    let (items, _) = crate::api::commands::models::load_live_catalog(leaf)?;
    let mut out = Vec::new();
    for item in items {
        let Some(id) = item["model_sku"].as_str().filter(|s| !s.is_empty()) else {
            continue;
        };
        out.push(Model {
            id: id.into(),
            name: id.into(),
            context: 131_072,
            output: 8_192,
            tools: item["tools_supported"].as_bool().unwrap_or(false),
            price: as_price(&item["price_m_output_tokens_usd"]),
        });
    }
    if out.is_empty() {
        bail!("key catalog contained no models");
    }
    Ok(out)
}
fn catalog(base: &str, has_key: bool) -> Result<Vec<Model>> {
    let feed = fetch_feed(base);
    if !has_key {
        return feed;
    }
    let key = key_catalog();
    match (feed, key) {
        (Ok(feed), Ok(visible)) => {
            let mut by_id: BTreeMap<String, Model> = feed.into_iter().map(|m| (m.id.clone(), m)).collect();
            let mut out = Vec::new();
            for item in visible {
                out.push(by_id.remove(&item.id).unwrap_or(item));
            }
            Ok(out)
        }
        (Ok(feed), Err(_)) => Ok(feed),
        (Err(_), Ok(visible)) => Ok(visible),
        (Err(a), Err(b)) => bail!("model feed failed: {a}; key catalog failed: {b}"),
    }
}
fn choose_default(models: &[Model], requested: Option<&str>) -> Result<String> {
    if let Some(id) = requested {
        if !models.iter().any(|m| m.id == id) {
            return Err(exit::usage(format!(
                "default model {id:?} is not in the available catalog"
            )));
        }
        return Ok(id.into());
    }
    let pool: Vec<&Model> = models.iter().filter(|m| m.tools).collect();
    let pool: Vec<&Model> = if pool.is_empty() { models.iter().collect() } else { pool };
    let best = pool
        .into_iter()
        .max_by(|a, b| match (a.price, b.price) {
            (Some(x), Some(y)) => x.total_cmp(&y).then(a.id.cmp(&b.id)),
            (Some(_), None) => std::cmp::Ordering::Greater,
            (None, Some(_)) => std::cmp::Ordering::Less,
            (None, None) => a.context.cmp(&b.context).then(a.id.cmp(&b.id)),
        })
        .context("no models")?;
    Ok(best.id.clone())
}
fn diff(old: &[String], new: &[String]) -> (Vec<String>, Vec<String>) {
    let a: BTreeSet<_> = old.iter().cloned().collect();
    let b: BTreeSet<_> = new.iter().cloned().collect();
    (b.difference(&a).cloned().collect(), a.difference(&b).cloned().collect())
}
#[allow(clippy::too_many_arguments)]
fn action(
    report: &mut Report,
    agent: &str,
    status: &str,
    path: &Path,
    old: &[String],
    new: &[String],
    note: Option<String>,
    command: Option<String>,
) {
    let (added, removed) = diff(old, new);
    report.actions.push(Action {
        agent: agent.into(),
        status: status.into(),
        path: path.display().to_string(),
        added,
        removed,
        note,
        command,
    });
}
fn has_json_comments(raw: &str) -> bool {
    let mut chars = raw.chars().peekable();
    let mut in_string = false;
    let mut escape = false;
    while let Some(c) = chars.next() {
        if in_string {
            if escape {
                escape = false;
            } else if c == '\\' {
                escape = true;
            } else if c == '"' {
                in_string = false;
            }
        } else if c == '"' {
            in_string = true;
        } else if c == '/' && matches!(chars.peek(), Some('/' | '*')) {
            return true;
        }
    }
    false
}

fn codex(
    report: &mut Report,
    path: &Path,
    models: &[Model],
    default: &str,
    rec: &mut Managed,
    opts: &Options,
) -> Result<bool> {
    let profile = path.with_file_name("mixlayer.config.toml");
    let original = read_optional(path)?.unwrap_or_default();
    let mut doc: DocumentMut = original
        .parse()
        .with_context(|| format!("parsing {}", path.display()))?;
    let current = doc
        .get("model_providers")
        .and_then(|v| v.get("mixlayer"))
        .map(ToString::to_string);
    let profile_current = read_optional(&profile)?;
    let edited = rec.installed
        && (current.as_deref().map(hash).as_deref() != Some(rec.hash.as_str())
            || profile_current.as_deref().map(hash).as_deref() != Some(rec.profile_hash.as_str()));
    if edited && !opts.force {
        action(
            report,
            "codex",
            "modified_skip",
            path,
            &rec.ids,
            &rec.ids,
            Some("managed provider or profile was edited".into()),
            None,
        );
        return Ok(false);
    }
    if opts.uninstall {
        if !rec.installed {
            action(report, "codex", "not_managed", path, &[], &[], None, None);
            return Ok(false);
        }
        if opts.dry_run {
            action(report, "codex", "would_remove", path, &rec.ids, &[], None, None);
            return Ok(false);
        }
        if let Some(table) = doc.get_mut("model_providers").and_then(Item::as_table_mut) {
            table.remove("mixlayer");
        }
        if doc
            .get("model_providers")
            .and_then(Item::as_table)
            .is_some_and(|t| t.is_empty())
        {
            doc.as_table_mut().remove("model_providers");
        }
        if rec.created && doc.to_string().trim().is_empty() {
            if path.exists() {
                backup_remove(path)?;
            }
        } else {
            write_file(path, &doc.to_string())?;
        }
        if rec.profile_created && profile.exists() {
            backup_remove(&profile)?;
        }
        let old = rec.ids.clone();
        *rec = Managed::default();
        action(report, "codex", "removed", path, &old, &[], None, None);
        return Ok(true);
    }
    if (current.is_some() || profile_current.is_some()) && !rec.installed {
        action(
            report,
            "codex",
            "conflict_skip",
            path,
            &[],
            &[],
            Some("provider.mixlayer already exists".into()),
            None,
        );
        return Ok(false);
    }
    let exe = std::env::current_exe()?;
    let mut provider = Table::new();
    provider["name"] = value("Mixlayer");
    provider["base_url"] = value(config::load(None)?.base_url);
    provider["wire_api"] = value("responses");
    let mut auth = Table::new();
    auth["command"] = value(exe.display().to_string());
    auth["args"] = value(toml_edit::Array::from_iter(["auth", "token"]));
    auth["timeout_ms"] = value(5000);
    provider["auth"] = Item::Table(auth);
    if doc.get("model_providers").is_none() {
        doc["model_providers"] = Item::Table(Table::new());
    }
    doc["model_providers"]["mixlayer"] = Item::Table(provider);
    let rendered = doc.to_string();
    let block = doc["model_providers"]["mixlayer"].to_string();
    let selected = models
        .iter()
        .find(|m| m.id == default)
        .context("default model unavailable")?;
    let profile_text = format!(
        "model_provider = \"mixlayer\"\nmodel = {}\nmodel_context_window = {}\n",
        serde_json::to_string(default)?,
        selected.context
    );
    let ids: Vec<_> = models.iter().map(|m| m.id.clone()).collect();
    if rec.installed
        && current.as_deref() == Some(block.as_str())
        && profile_current.as_deref() == Some(profile_text.as_str())
    {
        action(report, "codex", "unchanged", path, &rec.ids, &ids, None, None);
        return Ok(false);
    }
    if opts.dry_run {
        action(
            report,
            "codex",
            if rec.installed {
                "would_update"
            } else {
                "would_register"
            },
            path,
            &rec.ids,
            &ids,
            None,
            None,
        );
        return Ok(false);
    }
    let created = !path.exists();
    let profile_created = !profile.exists();
    write_file(path, &rendered)?;
    write_file(&profile, &profile_text)?;
    let old = rec.ids.clone();
    *rec = Managed {
        installed: true,
        hash: hash(&block),
        profile_hash: hash(&profile_text),
        created: rec.created || created,
        profile_created: rec.profile_created || profile_created,
        ids: ids.clone(),
    };
    action(
        report,
        "codex",
        if old.is_empty() { "registered" } else { "updated" },
        path,
        &old,
        &ids,
        Some("Use codex --profile mixlayer".into()),
        None,
    );
    Ok(true)
}

fn backup_remove(path: &Path) -> Result<()> {
    let backup = format!(
        "{}.bak-{}",
        path.display(),
        chrono::Utc::now().format("%Y%m%dT%H%M%S%.9fZ")
    );
    fs::copy(path, backup)?;
    fs::remove_file(path)?;
    Ok(())
}
fn opencode_provider(models: &[Model], base: &str) -> Value {
    let mut items = Map::new();
    for m in models {
        items.insert(
            m.id.clone(),
            json!({"name":m.name,"limit":{"context":m.context,"output":m.output}}),
        );
    }
    json!({"npm":"@ai-sdk/openai-compatible","name":"Mixlayer","options":{"baseURL":base,"apiKey":"{env:MIXLAYER_API_KEY}"},"models":items})
}
fn opencode(
    report: &mut Report,
    path: &Path,
    models: &[Model],
    base: &str,
    rec: &mut Managed,
    opts: &Options,
) -> Result<bool> {
    let raw = read_optional(path)?.unwrap_or_else(|| "{}\n".into());
    let provider = opencode_provider(models, base);
    if has_json_comments(&raw) {
        let snippet = format!("\"mixlayer\": {}", serde_json::to_string_pretty(&provider)?);
        action(
            report,
            "opencode",
            "conflict_skip",
            path,
            &rec.ids,
            &rec.ids,
            Some(format!("JSONC comments present; paste this under provider: {snippet}")),
            None,
        );
        return Ok(false);
    }
    let mut doc: Value = serde_json::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?;
    let root = doc.as_object_mut().context("OpenCode config must be a JSON object")?;
    let current = root.get("provider").and_then(|p| p.get("mixlayer")).cloned();
    let edited = rec.installed && current.as_ref().map(|v| hash(&v.to_string())).as_deref() != Some(rec.hash.as_str());
    if edited && !opts.force {
        action(
            report,
            "opencode",
            "modified_skip",
            path,
            &rec.ids,
            &rec.ids,
            None,
            None,
        );
        return Ok(false);
    }
    if opts.uninstall {
        if !rec.installed {
            action(report, "opencode", "not_managed", path, &[], &[], None, None);
            return Ok(false);
        }
        if opts.dry_run {
            action(report, "opencode", "would_remove", path, &rec.ids, &[], None, None);
            return Ok(false);
        }
        if let Some(providers) = root.get_mut("provider").and_then(Value::as_object_mut) {
            providers.remove("mixlayer");
            if providers.is_empty() {
                root.remove("provider");
            }
        }
        if rec.created && root.is_empty() {
            if path.exists() {
                backup_remove(path)?;
            }
        } else {
            write_file(path, &format!("{}\n", serde_json::to_string_pretty(&doc)?))?;
        }
        let old = rec.ids.clone();
        *rec = Managed::default();
        action(report, "opencode", "removed", path, &old, &[], None, None);
        return Ok(true);
    }
    if current.is_some() && !rec.installed {
        action(report, "opencode", "conflict_skip", path, &[], &[], None, None);
        return Ok(false);
    }
    let ids: Vec<_> = models.iter().map(|m| m.id.clone()).collect();
    if current.as_ref() == Some(&provider) && rec.installed {
        action(report, "opencode", "unchanged", path, &rec.ids, &ids, None, None);
        return Ok(false);
    }
    if opts.dry_run {
        action(
            report,
            "opencode",
            if rec.installed {
                "would_update"
            } else {
                "would_register"
            },
            path,
            &rec.ids,
            &ids,
            None,
            None,
        );
        return Ok(false);
    }
    let created = !path.exists();
    let providers = root
        .entry("provider")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .context("provider must be an object")?;
    providers.insert("mixlayer".into(), provider.clone());
    // Preserve $schema at the front for tools that expect it there.
    if let Some(schema) = root.shift_remove("$schema") {
        root.shift_insert(0, "$schema".into(), schema);
    }
    write_file(path, &format!("{}\n", serde_json::to_string_pretty(&doc)?))?;
    let old = rec.ids.clone();
    *rec = Managed {
        installed: true,
        hash: hash(&provider.to_string()),
        created: rec.created || created,
        ids: ids.clone(),
        ..Managed::default()
    };
    action(
        report,
        "opencode",
        if old.is_empty() { "registered" } else { "updated" },
        path,
        &old,
        &ids,
        None,
        None,
    );
    Ok(true)
}

fn hermes(
    report: &mut Report,
    path: &Path,
    models: &[Model],
    base: &str,
    rec: &mut Managed,
    opts: &Options,
) -> Result<bool> {
    let mut model_map = Map::new();
    for m in models {
        model_map.insert(m.id.clone(), json!({"context_length":m.context}));
    }
    let mapping = json!({"name":"Mixlayer","api":base,"key_env":"MIXLAYER_API_KEY","transport":"openai_chat","discover_models":false,"models":model_map});
    let ids: Vec<_> = models.iter().map(|m| m.id.clone()).collect();
    let command = format!(
        "hermes config set providers.mixlayer '{}'",
        mapping.to_string().replace('\'', "'\\''")
    );
    if opts.dry_run {
        action(
            report,
            "hermes",
            if opts.uninstall {
                "would_remove"
            } else if rec.installed {
                "would_update"
            } else {
                "would_register"
            },
            path,
            &rec.ids,
            if opts.uninstall { &[] } else { &ids },
            None,
            Some(command),
        );
        return Ok(false);
    }
    let installed = Command::new("hermes")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success());
    if !installed {
        action(
            report,
            "hermes",
            "not_installed",
            path,
            &rec.ids,
            &rec.ids,
            Some("Install Hermes, then run the command shown".into()),
            Some(command),
        );
        return Ok(false);
    }
    if rec.installed {
        let got = Command::new("hermes")
            .args(["config", "get", "providers.mixlayer", "--json"])
            .output()?;
        let current: Option<Value> = got
            .status
            .success()
            .then(|| serde_json::from_slice(&got.stdout).ok())
            .flatten();
        if current.as_ref().map(|v| hash(&v.to_string())).as_deref() != Some(rec.hash.as_str()) && !opts.force {
            action(report, "hermes", "modified_skip", path, &rec.ids, &rec.ids, None, None);
            return Ok(false);
        }
        if !opts.uninstall && current.as_ref() == Some(&mapping) {
            action(report, "hermes", "unchanged", path, &rec.ids, &ids, None, None);
            return Ok(false);
        }
    }
    if opts.uninstall && !rec.installed {
        action(report, "hermes", "not_managed", path, &[], &[], None, None);
        return Ok(false);
    }
    let output = if opts.uninstall {
        Command::new("hermes")
            .args(["config", "unset", "providers.mixlayer"])
            .output()?
    } else {
        Command::new("hermes")
            .args(["config", "set", "providers.mixlayer", &mapping.to_string()])
            .output()?
    };
    if !output.status.success() {
        bail!("hermes config failed: {}", String::from_utf8_lossy(&output.stderr));
    }
    let old = rec.ids.clone();
    if opts.uninstall {
        *rec = Managed::default();
        action(report, "hermes", "removed", path, &old, &[], None, None);
    } else {
        *rec = Managed {
            installed: true,
            hash: hash(&mapping.to_string()),
            ids: ids.clone(),
            ..Managed::default()
        };
        action(
            report,
            "hermes",
            if old.is_empty() { "registered" } else { "updated" },
            path,
            &old,
            &ids,
            Some(format!(
                "Use hermes chat --provider mixlayer --model {} or /model inside Hermes",
                ids.first().map(String::as_str).unwrap_or("<id>")
            )),
            None,
        );
    }
    Ok(true)
}

pub fn run(opts: Options) -> Result<Report> {
    if opts.agent != "all" && !AGENTS.contains(&opts.agent.as_str()) {
        return Err(exit::usage(format!("unknown agent {:?}", opts.agent)));
    }
    let home = home()?;
    let state_file = state_path(&home);
    let mut state = read_state(&state_file)?;
    let targets: Vec<&str> = if opts.agent == "all" {
        AGENTS
            .iter()
            .copied()
            .filter(|a| detected(a, &home) || managed(&state, a).installed)
            .collect()
    } else {
        vec![opts.agent.as_str()]
    };
    let mut report = Report {
        dry_run: opts.dry_run,
        uninstall: opts.uninstall,
        actions: Vec::new(),
    };
    let cfg = config::load(None).map_err(|e| exit::err(Code::Config, e.to_string()))?;
    let models = if opts.dry_run
        || opts.uninstall
        || targets
            .iter()
            .all(|a| matches!(*a, "cursor" | "claude-code" | "claude-desktop"))
    {
        Vec::new()
    } else {
        catalog(&cfg.base_url, !cfg.auth_header.is_empty()).map_err(|e| exit::err(Code::Api, e.to_string()))?
    };
    let default = if models.is_empty() {
        String::new()
    } else {
        choose_default(&models, opts.default_model.as_deref())?
    };
    if !models.is_empty() && !opts.dry_run {
        write_file(
            &feed_path(&home),
            &format!(
                "{}\n",
                json!({"ids":models.iter().map(|m|m.id.clone()).collect::<Vec<_>>()})
            ),
        )?;
    }
    for agent in targets {
        let path = config_path(agent, &home);
        let mut rec = managed(&state, agent);
        if opts.dry_run && matches!(agent, "codex" | "opencode" | "hermes") {
            action(
                &mut report,
                agent,
                if opts.uninstall {
                    if rec.installed { "would_remove" } else { "not_managed" }
                } else if rec.installed {
                    "would_update"
                } else {
                    "would_register"
                },
                &path,
                &rec.ids,
                if opts.uninstall { &[] } else { &rec.ids },
                Some("Preview only; no model feed fetched".into()),
                None,
            );
            continue;
        }
        let did_change = match agent {
            "codex" => codex(&mut report, &path, &models, &default, &mut rec, &opts)?,
            "opencode" => opencode(&mut report, &path, &models, &cfg.base_url, &mut rec, &opts)?,
            "hermes" => hermes(&mut report, &path, &models, &cfg.base_url, &mut rec, &opts)?,
            "cursor" => {
                action(
                    &mut report,
                    agent,
                    "instructions",
                    &path,
                    &[],
                    &[],
                    Some(format!(
                        "Settings → Models → OpenAI API key; override base URL to {}, then add Mixlayer model IDs",
                        cfg.base_url
                    )),
                    None,
                );
                false
            }
            "claude-code" | "claude-desktop" => {
                action(
                    &mut report,
                    agent,
                    "instructions",
                    &path,
                    &[],
                    &[],
                    Some("Needs Anthropic Messages API, not available yet".into()),
                    None,
                );
                false
            }
            _ => unreachable!(),
        };
        if did_change {
            set_managed(&mut state, agent, &rec)?;
            save_state(&state_file, &state)?;
        }
    }
    Ok(report)
}

pub fn doctor_report() -> Vec<Value> {
    let Ok(home) = home() else {
        return Vec::new();
    };
    let Ok(state) = read_state(&state_path(&home)) else {
        return Vec::new();
    };
    let cached: Value = fs::read(feed_path(&home))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(Value::Null);
    let feed: BTreeSet<String> = cached
        .get("ids")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();
    AGENTS.iter().map(|agent| {
        let rec = managed(&state,agent);
        let ids: BTreeSet<String> = rec.ids.iter().cloned().collect();
        json!({"agent":agent,"registered":rec.installed,"model_count":ids.len(),"differs_from_cached_feed":rec.installed && !feed.is_empty() && ids!=feed})
    }).collect()
}

pub fn render_human(report: &Report) -> String {
    let mut out = String::new();
    for action in &report.actions {
        out.push_str(&format!("{}: {}  {}\n", action.agent, action.status, action.path));
        if !action.added.is_empty() {
            out.push_str(&format!("  added: {}\n", action.added.join(", ")));
        }
        if !action.removed.is_empty() {
            out.push_str(&format!("  removed: {}\n", action.removed.join(", ")));
        }
        if let Some(note) = &action.note {
            out.push_str(&format!("  {note}\n"));
        }
        if let Some(command) = &action.command {
            out.push_str(&format!("  run: {command}\n"));
        }
    }
    if report.actions.is_empty() {
        out.push_str("No installed agent harnesses found.\n");
    }
    out
}
