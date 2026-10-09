// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

//! Where the CLI keeps config, data, state and cache. Same resolution order
//! and same directories as the Go CLI, so existing ledgers, vaults and
//! credentials keep working after the switch:
//!
//! 1. `MIXLAYER_<KIND>_DIR`
//! 2. `--home <dir>` → `<dir>/<kind>`
//! 3. `MIXLAYER_HOME` → `$MIXLAYER_HOME/<kind>`
//! 4. `XDG_<KIND>_HOME` → `$XDG_<KIND>_HOME/mixlayer`
//! 5. `~/.config`, `~/.local/share`, `~/.local/state`, `~/.cache` + `/mixlayer`
//!
//! Relative overrides are ignored with a one-time warning.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock, RwLock};

const APP: &str = "mixlayer";
const ENV_PREFIX: &str = "MIXLAYER";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Config,
    Data,
    State,
    Cache,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Config => "config",
            Kind::Data => "data",
            Kind::State => "state",
            Kind::Cache => "cache",
        }
    }
    fn env_suffix(self) -> &'static str {
        match self {
            Kind::Config => "CONFIG",
            Kind::Data => "DATA",
            Kind::State => "STATE",
            Kind::Cache => "CACHE",
        }
    }
    pub fn env_var(self) -> String {
        format!("{ENV_PREFIX}_{}_DIR", self.env_suffix())
    }
    fn xdg_var(self) -> String {
        format!("XDG_{}_HOME", self.env_suffix())
    }
    fn default_base(self, home: &Path) -> PathBuf {
        match self {
            Kind::Config => home.join(".config"),
            Kind::Data => home.join(".local").join("share"),
            Kind::State => home.join(".local").join("state"),
            Kind::Cache => home.join(".cache"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolution {
    pub kind: Kind,
    pub dir: PathBuf,
    /// Which rule won: per-kind-env, --home, home-env, xdg-env, platform-default.
    pub rung: &'static str,
    /// The env var or flag that supplied the value.
    pub source: String,
    pub ignored: Vec<(String, String)>,
}

fn home_override() -> &'static RwLock<Option<PathBuf>> {
    static H: OnceLock<RwLock<Option<PathBuf>>> = OnceLock::new();
    H.get_or_init(|| RwLock::new(None))
}

/// Applies `--home`. The path must be absolute and, if it exists, a directory.
pub fn set_home_override(raw: &str) -> anyhow::Result<()> {
    let raw = raw.trim();
    let value = if raw.is_empty() {
        None
    } else {
        let clean =
            clean_override(raw).ok_or_else(|| anyhow::anyhow!("invalid --home {raw:?}: path must be absolute"))?;
        match std::fs::metadata(&clean) {
            Ok(m) if !m.is_dir() => anyhow::bail!("--home {:?}: not a directory", clean.display().to_string()),
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => anyhow::bail!("checking --home {:?}: {e}", clean.display().to_string()),
        }
        Some(clean)
    };
    *home_override().write().expect("home override lock") = value;
    Ok(())
}

pub fn dir(kind: Kind) -> anyhow::Result<PathBuf> {
    Ok(resolve(kind)?.dir)
}

pub fn resolve(kind: Kind) -> anyhow::Result<Resolution> {
    let mut ignored = Vec::new();
    let res = |dir: PathBuf, rung: &'static str, source: String, ignored: Vec<(String, String)>| Resolution {
        kind,
        dir,
        rung,
        source,
        ignored,
    };
    let per_kind = kind.env_var();
    match env_dir(&per_kind) {
        EnvDir::Set(d) => return Ok(res(d, "per-kind-env", per_kind, ignored)),
        EnvDir::Ignored(v) => ignored.push((per_kind, v)),
        EnvDir::Unset => {}
    }
    if let Some(h) = home_override().read().expect("home override lock").clone() {
        return Ok(res(h.join(kind.name()), "--home", "--home".into(), ignored));
    }
    let home_var = format!("{ENV_PREFIX}_HOME");
    match env_dir(&home_var) {
        EnvDir::Set(d) => return Ok(res(d.join(kind.name()), "home-env", home_var, ignored)),
        EnvDir::Ignored(v) => ignored.push((home_var, v)),
        EnvDir::Unset => {}
    }
    let xdg = kind.xdg_var();
    match env_dir(&xdg) {
        EnvDir::Set(d) => return Ok(res(d.join(APP), "xdg-env", xdg, ignored)),
        EnvDir::Ignored(v) => ignored.push((xdg, v)),
        EnvDir::Unset => {}
    }
    let home = user_home().ok_or_else(|| anyhow::anyhow!("resolve user home dir: not found"))?;
    Ok(res(
        kind.default_base(&home).join(APP),
        "platform-default",
        "platform-default".into(),
        ignored,
    ))
}

enum EnvDir {
    Unset,
    Set(PathBuf),
    Ignored(String),
}

fn env_dir(name: &str) -> EnvDir {
    let raw = std::env::var(name).unwrap_or_default();
    let raw = raw.trim();
    if raw.is_empty() {
        return EnvDir::Unset;
    }
    match clean_override(raw) {
        Some(p) => EnvDir::Set(p),
        None => {
            warn_once(name, raw);
            EnvDir::Ignored(raw.to_string())
        }
    }
}

fn warn_once(name: &str, raw: &str) {
    static SEEN: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    let key = format!("{name}\0{raw}");
    let mut seen = SEEN.get_or_init(Default::default).lock().expect("warn lock");
    if seen.insert(key) {
        eprintln!("warning: ignoring {name}={raw:?}: path must be absolute");
    }
}

/// Trims, expands a leading `~`, and requires an absolute path.
pub fn clean_override(raw: &str) -> Option<PathBuf> {
    let raw = raw.trim();
    let expanded = if raw == "~" {
        user_home()?
    } else if let Some(rest) = raw.strip_prefix("~/") {
        user_home()?.join(rest)
    } else {
        PathBuf::from(raw)
    };
    if raw.is_empty() || !expanded.is_absolute() {
        return None;
    }
    Some(normalize(&expanded))
}

/// Lexical cleanup like Go's filepath.Clean: drops `.` and resolves `..`
/// without touching the filesystem.
fn normalize(p: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

pub fn user_home() -> Option<PathBuf> {
    #[allow(deprecated)] // std::env::home_dir is fixed on all supported platforms since 1.86
    std::env::home_dir().filter(|p| !p.as_os_str().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_requires_absolute() {
        assert!(clean_override("relative/dir").is_none());
        #[cfg(unix)]
        assert_eq!(clean_override("/a/./b/../c"), Some(PathBuf::from("/a/c")));
        #[cfg(windows)]
        assert_eq!(clean_override(r"C:\a\.\b\..\c"), Some(PathBuf::from(r"C:\a\c")));
        // A rooted path without a drive is not absolute on Windows (as in Go).
        #[cfg(windows)]
        assert!(clean_override("/a/c").is_none());
        assert!(clean_override("~").is_some());
    }

    #[test]
    fn env_var_names_match_go() {
        assert_eq!(Kind::Data.env_var(), "MIXLAYER_DATA_DIR");
        assert_eq!(Kind::Cache.xdg_var(), "XDG_CACHE_HOME");
    }
}
