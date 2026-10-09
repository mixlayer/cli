use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context;
use toml::Value;

use crate::paths::{self, Kind};

#[derive(Clone, Debug)]
pub struct Config {
    pub path: PathBuf,
    pub credentials_path: PathBuf,
    pub base_url: String,
    pub auth_header: String,
    pub auth_source: String,
    pub credential_source: String,
    pub headers: Vec<(String, String)>,
    pub(crate) legacy_path: Option<PathBuf>,
}

fn read_toml(path: &Path) -> anyhow::Result<Option<Value>> {
    match fs::read_to_string(path) {
        Ok(data) => Ok(Some(
            toml::from_str::<Value>(&data).with_context(|| format!("parsing {}", path.display()))?,
        )),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

fn field<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}

fn has_creds(value: &Value) -> bool {
    [
        "auth_header",
        "access_token",
        "api_key",
        "refresh_token",
        "client_id",
        "client_secret",
    ]
    .iter()
    .any(|key| !field(value, key).is_empty())
}

pub fn load(explicit: Option<&str>) -> anyhow::Result<Config> {
    let path = explicit
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var("MIXLAYER_CONFIG")
                .ok()
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
        })
        .unwrap_or(paths::dir(Kind::Config)?.join("config.toml"));
    let credentials_path = paths::dir(Kind::Data)?.join("credentials.toml");
    let legacy = paths::user_home().map(|h| h.join(".config/mixlayer/config.toml"));
    let mut legacy_path = None;
    let mut config_value = read_toml(&path)?;
    if config_value.is_none()
        && explicit.is_none()
        && std::env::var_os("MIXLAYER_CONFIG").is_none()
        && let Some(other) = legacy.as_ref().filter(|p| **p != path)
    {
        match read_toml(other) {
            Ok(Some(v)) => {
                config_value = Some(v);
                legacy_path = Some(other.clone());
            }
            Ok(None) => {}
            Err(e) => eprintln!("warning: legacy config parse skipped for {}: {e}", other.display()),
        }
    }
    let mut base_url = config_value
        .as_ref()
        .map(|v| field(v, "base_url"))
        .filter(|s| !s.is_empty())
        .unwrap_or("https://mixlayer.ai/v1")
        .to_owned();
    let mut source = String::new();
    let mut credential_source = String::new();
    let mut auth = String::new();
    let mut headers = Vec::new();
    if let Some(v) = &config_value
        && let Some(h) = v.get("headers").and_then(Value::as_table)
    {
        headers = h
            .iter()
            .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_owned())))
            .collect();
    }
    let mut creds = config_value.as_ref();
    let file_creds = match read_toml(&credentials_path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!(
                "warning: credentials parse failed for {}: {e}",
                credentials_path.display()
            );
            None
        }
    };
    if let Some(v) = file_creds.as_ref().filter(|v| has_creds(v)) {
        creds = Some(v);
        credential_source = "credentials file".into();
    }
    if let Some(v) = creds {
        let head = field(v, "auth_header");
        let key = field(v, "api_key");
        let access = field(v, "access_token");
        auth = if !head.is_empty() {
            head.to_owned()
        } else if !key.is_empty() {
            format!("Bearer {key}")
        } else if !access.is_empty() {
            format!("Bearer {access}")
        } else {
            String::new()
        };
        if has_creds(v) {
            source = "config".into();
            if credential_source.is_empty() {
                credential_source = if legacy_path.is_some() {
                    "legacy config path"
                } else {
                    "config-kind path"
                }
                .into();
            }
        }
    }
    if let Ok(key) = std::env::var("MIXLAYER_API_KEY")
        && !key.is_empty()
    {
        // A legacy auth_header in config has always had priority over this env var.
        if creds.is_none_or(|v| field(v, "auth_header").is_empty()) {
            auth = format!("Bearer {key}");
            source = "env:MIXLAYER_API_KEY".into();
            credential_source = source.clone();
        }
    }
    if let Ok(value) = std::env::var("MIXLAYER_BASE_URL")
        && !value.is_empty()
    {
        base_url = value;
    }
    Ok(Config {
        path,
        credentials_path,
        base_url,
        auth_header: auth,
        auth_source: source,
        credential_source,
        headers,
        legacy_path,
    })
}

fn atomic_private(path: &Path, content: &str) -> anyhow::Result<()> {
    use std::io::Write;
    let parent = path.parent().context("credential path has no parent")?;
    create_private_dir(parent)?;
    let mut nonce = [0_u8; 8];
    getrandom::fill(&mut nonce)?;
    let tmp = path.with_extension(format!("tmp.{}.{}", std::process::id(), hex::encode(nonce)));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&tmp)?;
    let result = (|| -> anyhow::Result<()> {
        file.write_all(content.as_bytes())?;
        file.sync_all()?;
        fs::rename(&tmp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

pub(crate) fn create_private_dir(path: &Path) -> anyhow::Result<()> {
    if path.as_os_str().is_empty() {
        return Ok(());
    }
    if path.is_dir() {
        return Ok(());
    }
    if path.exists() {
        anyhow::bail!("{} is not a directory", path.display());
    }
    if let Some(parent) = path.parent() {
        create_private_dir(parent)?;
    }
    match fs::create_dir(path) {
        Ok(()) => {
            #[cfg(unix)]
            fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o700))?;
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && path.is_dir() => Ok(()),
        Err(e) => Err(e.into()),
    }
}

fn scrub(value: &mut Value) {
    if let Some(t) = value.as_table_mut() {
        for key in [
            "auth_header",
            "api_key",
            "access_token",
            "refresh_token",
            "token_expiry",
            "client_id",
            "client_secret",
        ] {
            t.remove(key);
        }
    }
}

impl Config {
    pub fn save_token(&self, token: &str) -> anyhow::Result<()> {
        let mut creds = toml::map::Map::new();
        creds.insert("access_token".into(), Value::String(token.into()));
        atomic_private(&self.credentials_path, &toml::to_string(&Value::Table(creds))?)?;
        self.scrub_config()
    }

    pub fn logout(&self) -> anyhow::Result<()> {
        match fs::remove_file(&self.credentials_path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
            _ => {}
        }
        self.scrub_config()
    }

    fn scrub_config(&self) -> anyhow::Result<()> {
        for path in std::iter::once(&self.path).chain(self.legacy_path.iter()) {
            let mut value = read_toml(path)?.unwrap_or_else(|| Value::Table(toml::map::Map::new()));
            scrub(&mut value);
            if let Some(t) = value.as_table_mut()
                && !t.contains_key("base_url")
            {
                t.insert("base_url".into(), Value::String(self.base_url.clone()));
            }
            atomic_private(path, &toml::to_string(&value)?)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrub_removes_all_legacy_credential_fields() {
        let mut value: Value = toml::from_str("base_url = 'https://example.test/v1'\nauth_header = 'Basic secret'\naccess_token = 'secret'\napi_key = 'secret'\n[headers]\nAccept = 'application/json'\n").unwrap();
        scrub(&mut value);
        assert_eq!(field(&value, "base_url"), "https://example.test/v1");
        assert!(!has_creds(&value));
        assert_eq!(value["headers"]["Accept"].as_str(), Some("application/json"));
    }
}
