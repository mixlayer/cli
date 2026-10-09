// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

//! Saved flag profiles in `<config>/profiles.json` (same file and shape as
//! the Go CLI; falls back to reading `~/.mixlayer/profiles.json`).

use std::collections::BTreeMap;
use std::path::PathBuf;

use clap::ArgMatches;
use clap::parser::ValueSource;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::exit::{self, Code};
use crate::{Globals, output, paths};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Profile {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default)]
    pub values: BTreeMap<String, String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Store {
    #[serde(default)]
    pub profiles: BTreeMap<String, Profile>,
}

/// Flags a profile never sets: they control profile/config resolution.
/// `profile save` skips a superset of these so saved values always apply.
pub const RESERVED: &[&str] = &["profile", "config", "home", "help"];
const SAVE_SKIP: &[&str] = &["profile", "config", "home", "help", "description"];

pub fn store_path() -> anyhow::Result<PathBuf> {
    Ok(paths::dir(paths::Kind::Config)?.join("profiles.json"))
}

fn legacy_path() -> Option<PathBuf> {
    paths::user_home().map(|h| h.join(".mixlayer").join("profiles.json"))
}

pub fn load() -> anyhow::Result<Store> {
    let primary = store_path()?;
    let data = match std::fs::read(&primary) {
        Ok(d) => d,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            match legacy_path().filter(|l| *l != primary).map(std::fs::read) {
                Some(Ok(d)) => d,
                _ => return Ok(Store::default()),
            }
        }
        Err(e) => anyhow::bail!("reading profiles: {e}"),
    };
    serde_json::from_slice(&data).map_err(|e| anyhow::anyhow!("parsing profiles: {e}"))
}

fn save_store(s: &Store) -> anyhow::Result<()> {
    let p = store_path()?;
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(|e| anyhow::anyhow!("creating profile config dir: {e}"))?;
    }
    let data = serde_json::to_vec_pretty(s)?;
    let tmp = p.with_extension("json.tmp");
    write_private(&tmp, &data).map_err(|e| anyhow::anyhow!("writing profiles: {e}"))?;
    std::fs::rename(&tmp, &p)?;
    Ok(())
}

#[cfg(unix)]
fn write_private(p: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(p)?;
    f.write_all(data)
}

#[cfg(not(unix))]
fn write_private(p: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    std::fs::write(p, data)
}

pub fn names() -> Vec<String> {
    load().map(|s| s.profiles.into_keys().collect()).unwrap_or_default()
}

pub fn get(name: &str) -> anyhow::Result<Option<Profile>> {
    Ok(load()?.profiles.remove(name))
}

/// Resolves `--profile <name>`, with the Go CLI's not-found messages.
pub fn resolve(name: &str) -> anyhow::Result<Profile> {
    if let Some(p) = get(name)? {
        return Ok(p);
    }
    let available = names();
    if available.is_empty() {
        anyhow::bail!(
            "profile {name:?} not found (no profiles saved yet; run 'mixlayer profile save <name> --<flag> <value>')"
        );
    }
    anyhow::bail!("profile {name:?} not found; available: {}", available.join(", "))
}

fn not_found(name: &str) -> anyhow::Error {
    exit::err(Code::Other, format!("profile {name:?} not found"))
}

fn first_positional(leaf: &ArgMatches, path: &str) -> Option<String> {
    let spec = crate::spec_types::find(path)?;
    crate::cli::positional_words(spec, leaf).into_iter().next()
}

/// Flag values the user set explicitly on this invocation, as strings.
fn explicit_values(leaf: &ArgMatches, path: &str) -> BTreeMap<String, String> {
    let positional: Vec<String> = crate::spec_types::find(path)
        .map(|c| c.positionals().into_iter().map(|p| p.name.clone()).collect())
        .unwrap_or_default();
    let mut out = BTreeMap::new();
    for id in leaf.ids() {
        let id = id.as_str();
        if SAVE_SKIP.contains(&id) || id == crate::cli::EXTRA_ARGS || positional.iter().any(|p| p == id) {
            continue;
        }
        if leaf.value_source(id) != Some(ValueSource::CommandLine) {
            continue;
        }
        let value = if let Ok(Some(b)) = leaf.try_get_one::<bool>(id) {
            b.to_string()
        } else if let Ok(Some(vals)) = leaf.try_get_raw(id) {
            vals.map(|v| v.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join(",")
        } else {
            continue;
        };
        out.insert(id.to_string(), value);
    }
    out
}

/// `mixlayer profile ...` commands. Returns None for other paths.
pub fn dispatch(path: &[&str], leaf: &ArgMatches, g: &Globals) -> Option<anyhow::Result<()>> {
    let sub = match path {
        ["profile", sub] => *sub,
        _ => return None,
    };
    let full = format!("profile {sub}");
    let name = first_positional(leaf, &full);
    let need_name = || {
        name.clone()
            .ok_or_else(|| exit::usage(format!("accepts 1 arg(s), received 0 (usage: mixlayer {full} <name>)")))
    };
    Some((|| match sub {
        "save" => {
            let name = need_name()?;
            if name.contains(['/', '\\', ':', ' ']) {
                anyhow::bail!("profile name {name:?} contains reserved characters");
            }
            let values = explicit_values(leaf, &full);
            if values.is_empty() {
                anyhow::bail!("no non-default flags set - pass at least one flag to save into {name:?}");
            }
            let description = leaf.get_one::<String>("description").cloned().unwrap_or_default();
            let mut s = load()?;
            let n = values.len();
            let p = Profile {
                name: name.clone(),
                description,
                values,
            };
            s.profiles.insert(name.clone(), p.clone());
            save_store(&s)?;
            if g.json {
                return output::print_json(&serde_json::to_value(&p)?, g);
            }
            println!("saved profile {name:?} with {n} values");
            Ok(())
        }
        "use" => {
            let name = need_name()?;
            let p = get(&name)?.ok_or_else(|| not_found(&name))?;
            if g.json {
                return output::print_json(&serde_json::to_value(&p)?, g);
            }
            let mut out = format!("profile {:?}:\n", p.name);
            if !p.description.is_empty() {
                out.push_str(&format!("  description: {}\n", p.description));
            }
            for (k, v) in &p.values {
                out.push_str(&format!("  --{k} {v}\n"));
            }
            output::write_stdout(out.as_bytes())
        }
        "show" => {
            let name = need_name()?;
            let p = get(&name)?.ok_or_else(|| not_found(&name))?;
            // Always JSON, and the description key is always present (as in Go).
            output::print_json(
                &json!({"name": p.name, "description": p.description, "values": p.values}),
                g,
            )
        }
        "list" => {
            let s = load()?;
            if g.json {
                let rows: Vec<_> = s
                    .profiles
                    .values()
                    .map(|p| json!({"description": p.description, "field_count": p.values.len(), "name": p.name}))
                    .collect();
                return output::print_json(&serde_json::Value::Array(rows), g);
            }
            let mut rows = vec![vec!["NAME".to_string(), "FIELDS".into(), "DESCRIPTION".into()]];
            for p in s.profiles.values() {
                rows.push(vec![p.name.clone(), p.values.len().to_string(), p.description.clone()]);
            }
            output::write_stdout(table(&rows).as_bytes())
        }
        "delete" => {
            let name = need_name()?;
            let mut s = load()?;
            if !s.profiles.contains_key(&name) {
                return Err(not_found(&name));
            }
            if !g.yes {
                eprintln!("refusing to delete {name:?} without --yes");
                anyhow::bail!("confirmation required: pass --yes");
            }
            s.profiles.remove(&name);
            save_store(&s)?;
            if g.json {
                return output::print_json(&json!({"deleted": name}), g);
            }
            println!("deleted profile {name:?}");
            Ok(())
        }
        _ => Err(exit::usage(format!("unknown command {sub:?} for \"mixlayer profile\""))),
    })())
}

/// Columns padded to their widest cell plus two spaces; the last column is
/// written as is (Go's tabwriter with padding 2).
fn table(rows: &[Vec<String>]) -> String {
    let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
    let widths: Vec<usize> = (0..cols)
        .map(|i| {
            rows.iter()
                .filter_map(|r| r.get(i))
                .map(|c| c.chars().count())
                .max()
                .unwrap_or(0)
        })
        .collect();
    let mut out = String::new();
    for r in rows {
        for (i, cell) in r.iter().enumerate() {
            if i + 1 == r.len() {
                out.push_str(cell);
            } else {
                out.push_str(&format!("{cell:<w$}", w = widths[i] + 2));
            }
        }
        out.push('\n');
    }
    out
}

/// Arguments to append so a profile's values apply to flags the user did not
/// set explicitly. Bool flags become `--flag`; others `--flag=value`.
pub fn overlay_args(profile: &Profile, cmd: &clap::Command, leaf: &ArgMatches) -> Vec<String> {
    let mut out = Vec::new();
    for (name, value) in &profile.values {
        if RESERVED.contains(&name.as_str()) {
            continue;
        }
        let Some(arg) = cmd
            .get_arguments()
            .find(|a| a.get_id().as_str() == name && a.get_long().is_some())
        else {
            continue;
        };
        if leaf.value_source(name) == Some(ValueSource::CommandLine) {
            continue;
        }
        let is_bool = matches!(arg.get_action(), clap::ArgAction::SetTrue | clap::ArgAction::SetFalse);
        if is_bool {
            if value == "true" {
                out.push(format!("--{name}"));
            }
        } else {
            out.push(format!("--{name}={value}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_pads_all_but_last_column() {
        let rows = vec![
            vec!["NAME".to_string(), "FIELDS".into(), "DESCRIPTION".into()],
            vec!["agentish".into(), "1".into(), String::new()],
        ];
        assert_eq!(table(&rows), "NAME      FIELDS  DESCRIPTION\nagentish  1       \n");
    }

    #[test]
    fn overlay_skips_reserved_explicit_and_unknown_flags() {
        let cmd = clap::Command::new("t")
            .arg(clap::Arg::new("json").long("json").action(clap::ArgAction::SetTrue))
            .arg(clap::Arg::new("select").long("select"))
            .arg(
                clap::Arg::new("compact")
                    .long("compact")
                    .action(clap::ArgAction::SetTrue),
            )
            .arg(clap::Arg::new("home").long("home"));
        let m = cmd.clone().get_matches_from(["t", "--select", "id"]);
        let p = Profile {
            name: "p".into(),
            description: String::new(),
            values: [
                ("json", "true"),
                ("select", "name"),
                ("compact", "false"),
                ("home", "/x"),
                ("model", "m"),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        };
        assert_eq!(overlay_args(&p, &cmd, &m), vec!["--json".to_string()]);
    }
}
