// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.
use serde_json::{Value, json};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::Path;
use std::process::Command;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::Duration;
const TEST_KEY: &str = concat!("MIXLAYER_TEST_", "KEY_VALUE");

struct Feed {
    url: String,
    models: Arc<Mutex<Value>>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Feed {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        let models = Arc::new(Mutex::new(
            json!({"data":[{"id":"a/model","name":"A","context_length":100000,"max_output_length":10000,"supported_features":["tools"],"pricing":{"completion":"0.01"}}]}),
        ));
        let stop = Arc::new(AtomicBool::new(false));
        let m = models.clone();
        let s = stop.clone();
        let handle = thread::spawn(move || {
            while !s.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream.set_nonblocking(false).unwrap();
                        stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                        let mut reader = BufReader::new(&mut stream);
                        let mut request_line = String::new();
                        reader.read_line(&mut request_line).unwrap();
                        loop {
                            let mut line = String::new();
                            if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                                break;
                            }
                        }
                        drop(reader);
                        let body = if request_line.contains("/_openrouter/models") {
                            m.lock().unwrap().to_string()
                        } else if request_line.contains("/v1/models") {
                            json!({"data":[{"id":"a/model"},{"id":"missing/model"}]}).to_string()
                        } else {
                            "[]".into()
                        };
                        let response = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            body.len(),
                            body
                        );
                        stream.write_all(response.as_bytes()).unwrap();
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => thread::sleep(Duration::from_millis(5)),
                    Err(_) => break,
                }
            }
        });
        Self {
            url,
            models,
            stop,
            thread: Some(handle),
        }
    }
    fn add(&self) {
        self.models.lock().unwrap()["data"].as_array_mut().unwrap().push(json!({"id":"b/model","name":"B","context_length":200000,"max_output_length":20000,"supported_features":["tools"],"pricing":{"completion":"0.02"}}));
    }
}
impl Drop for Feed {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.thread.take().unwrap().join().unwrap();
    }
}
fn env_command(home: &Path, feed: &Feed) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_mxl"));
    c.env("HOME", home)
        .env("USERPROFILE", home)
        .env("CODEX_HOME", home.join(".codex"))
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("HERMES_HOME", home.join(".hermes"))
        .env("MIXLAYER_HOME", home.join("mixlayer"))
        .env("MIXLAYER_BASE_URL", &feed.url)
        .env("MIXLAYER_PLATFORM_URL", feed.url.trim_end_matches("/v1"))
        .env("MIXLAYER_NO_AGENT_DETECT", "1")
        .env_remove("MIXLAYER_API_KEY");
    c
}
fn run(home: &Path, feed: &Feed, args: &[&str]) -> Value {
    let out = env_command(home, feed).args(args).output().unwrap();
    assert!(
        out.status.success(),
        "{} {:?}",
        String::from_utf8_lossy(&out.stderr),
        args
    );
    serde_json::from_slice(&out.stdout).unwrap()
}
fn status(report: &Value) -> &str {
    report["actions"][0]["status"].as_str().unwrap()
}
fn inspect_no_key(home: &Path) {
    fn walk(p: &Path) {
        if !p.exists() {
            return;
        }
        for entry in fs::read_dir(p).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path);
            } else {
                assert!(!String::from_utf8_lossy(&fs::read(&path).unwrap()).contains(TEST_KEY));
            }
        }
    }
    walk(home);
}
#[test]
fn codex_register_update_edit_and_uninstall() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path();
    let feed = Feed::new();
    fs::create_dir_all(home.join(".codex")).unwrap();
    let path = home.join(".codex/config.toml");
    fs::write(
        &path,
        "# keep this comment\n[model_providers.other]\nname = \"Other\"\n",
    )
    .unwrap();
    let args = ["models", "sync", "--agent", "codex", "--yes"];
    assert_eq!(status(&run(home, &feed, &args)), "registered");
    let content = fs::read_to_string(&path).unwrap();
    assert!(content.starts_with("# keep this comment\n[model_providers.other]\nname = \"Other\"\n"));
    assert!(content.contains("wire_api = \"responses\""));
    assert!(content.contains("[model_providers.mixlayer.auth]"));
    assert_eq!(status(&run(home, &feed, &args)), "unchanged");
    feed.add();
    let changed = run(home, &feed, &args);
    assert_eq!(status(&changed), "updated");
    assert_eq!(changed["actions"][0]["added"], json!(["b/model"]));
    let edited = fs::read_to_string(&path)
        .unwrap()
        .replace("wire_api = \"responses\"", "wire_api = \"chat\"");
    fs::write(&path, edited).unwrap();
    assert_eq!(status(&run(home, &feed, &args)), "modified_skip");
    let uninstall = run(
        home,
        &feed,
        &["models", "sync", "--agent", "codex", "--uninstall", "--yes"],
    );
    assert_eq!(status(&uninstall), "modified_skip");
    let forced = run(
        home,
        &feed,
        &["models", "sync", "--agent", "codex", "--uninstall", "--force", "--yes"],
    );
    assert_eq!(status(&forced), "removed");
    assert!(fs::read_to_string(&path).unwrap().contains("[model_providers.other]"));
    assert!(
        !fs::read_to_string(&path)
            .unwrap()
            .contains("[model_providers.mixlayer]")
    );
    inspect_no_key(home);
}
#[test]
fn opencode_register_update_edit_and_uninstall() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path();
    let feed = Feed::new();
    fs::create_dir_all(home.join(".config/opencode")).unwrap();
    let path = home.join(".config/opencode/opencode.json");
    fs::write(
        &path,
        "{\"$schema\":\"test\",\"provider\":{\"other\":{\"name\":\"Other\"}}}\n",
    )
    .unwrap();
    let args = ["models", "sync", "--agent", "opencode", "--yes"];
    assert_eq!(status(&run(home, &feed, &args)), "registered");
    let raw = fs::read_to_string(&path).unwrap();
    assert!(raw.find("\"$schema\"").unwrap() < raw.find("\"provider\"").unwrap());
    assert!(!raw.contains(TEST_KEY));
    assert_eq!(status(&run(home, &feed, &args)), "unchanged");
    feed.add();
    let changed = run(home, &feed, &args);
    assert_eq!(changed["actions"][0]["added"], json!(["b/model"]));
    let mut doc: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    doc["provider"]["mixlayer"]["name"] = "Edited".into();
    fs::write(&path, serde_json::to_string_pretty(&doc).unwrap()).unwrap();
    assert_eq!(status(&run(home, &feed, &args)), "modified_skip");
    assert_eq!(
        status(&run(
            home,
            &feed,
            &[
                "models",
                "sync",
                "--agent",
                "opencode",
                "--uninstall",
                "--force",
                "--yes"
            ]
        )),
        "removed"
    );
    let doc: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert!(doc["provider"]["other"].is_object());
    assert!(doc["provider"].get("mixlayer").is_none());
    inspect_no_key(home);
}
#[test]
fn preview_without_yes_reads_feed_and_writes_nothing() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path();
    let feed = Feed::new();
    fs::create_dir_all(home.join(".codex")).unwrap();
    fs::create_dir_all(home.join(".config/opencode")).unwrap();
    let report = run(home, &feed, &["models", "sync", "--agent", "all"]);
    assert_eq!(report["dry_run"], true);
    for agent in ["codex", "opencode"] {
        let a = report["actions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["agent"] == agent)
            .unwrap();
        assert_eq!(a["status"], "would_register", "{agent}: {a}");
        assert_eq!(a["added"], json!(["a/model"]), "{agent}: {a}");
    }
    assert!(!home.join(".codex/config.toml").exists());
    assert!(!home.join(".codex/mixlayer.config.toml").exists());
    assert!(!home.join(".config/opencode/opencode.json").exists());
    // After a real sync, a preview against the same feed is "unchanged",
    // and a new upstream model shows up as added.
    run(home, &feed, &["models", "sync", "--agent", "codex", "--yes"]);
    let again = run(home, &feed, &["models", "sync", "--agent", "codex"]);
    assert_eq!(again["actions"][0]["status"], "unchanged", "{again}");
    feed.add();
    let grown = run(home, &feed, &["models", "sync", "--agent", "codex"]);
    assert_eq!(grown["actions"][0]["status"], "would_update", "{grown}");
    assert_eq!(grown["actions"][0]["added"], json!(["b/model"]), "{grown}");
    let toml = fs::read_to_string(home.join(".codex/config.toml")).unwrap();
    assert!(!toml.contains("b/model"));
    let profile = fs::read_to_string(home.join(".codex/mixlayer.config.toml")).unwrap();
    assert!(!profile.contains("b/model"));
}
#[test]
fn codex_catalog_lists_models_with_real_limits_and_keeps_default() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path();
    let feed = Feed::new();
    feed.add();
    fs::create_dir_all(home.join(".codex")).unwrap();
    // The user picks the cheaper model as the profile default.
    run(
        home,
        &feed,
        &[
            "models",
            "sync",
            "--agent",
            "codex",
            "--default-model",
            "a/model",
            "--yes",
        ],
    );
    let catalog_path = home.join(".codex/mixlayer-models.json");
    let catalog: Value = serde_json::from_slice(&fs::read(&catalog_path).unwrap()).unwrap();
    let models = catalog["models"].as_array().unwrap();
    assert_eq!(models.len(), 2);
    // Default first, every model visible in /model, real context windows,
    // Codex's own base instructions.
    assert_eq!(models[0]["slug"], "a/model");
    for m in models {
        assert_eq!(m["visibility"], "list");
        assert!(
            m["base_instructions"]
                .as_str()
                .unwrap()
                .starts_with("You are a coding agent running in the Codex CLI")
        );
    }
    let b = models.iter().find(|m| m["slug"] == "b/model").unwrap();
    assert_eq!(b["context_window"], 200000);
    assert_eq!(b["max_context_window"], 200000);
    assert!(b["description"].as_str().unwrap().contains("200K context"));
    // The catalog is wired only into the mixlayer profile, never the base config.
    let profile = fs::read_to_string(home.join(".codex/mixlayer.config.toml")).unwrap();
    assert!(profile.contains("model = \"a/model\""), "{profile}");
    assert!(profile.contains("model_catalog_json"), "{profile}");
    assert!(!profile.contains("model_context_window"), "{profile}");
    let base = fs::read_to_string(home.join(".codex/config.toml")).unwrap();
    assert!(!base.contains("model_catalog_json"), "{base}");
    // A later plain sync keeps the user's default instead of the priciest model.
    let again = run(home, &feed, &["models", "sync", "--agent", "codex", "--yes"]);
    assert_eq!(again["actions"][0]["status"], "unchanged", "{again}");
    let profile = fs::read_to_string(home.join(".codex/mixlayer.config.toml")).unwrap();
    assert!(profile.contains("model = \"a/model\""), "{profile}");
    // An edited catalog is left alone; uninstall removes it.
    fs::write(&catalog_path, "{\"models\":[]}\n").unwrap();
    let edited = run(home, &feed, &["models", "sync", "--agent", "codex", "--yes"]);
    assert_eq!(edited["actions"][0]["status"], "modified_skip", "{edited}");
    run(
        home,
        &feed,
        &["models", "sync", "--agent", "codex", "--uninstall", "--force", "--yes"],
    );
    assert!(!catalog_path.exists());
    assert!(!home.join(".codex/mixlayer.config.toml").exists());
}
#[test]
fn dry_run_does_not_connect_or_write() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path();
    let feed = Feed::new();
    fs::create_dir_all(home.join(".codex")).unwrap();
    let before = fs::read_dir(home).unwrap().count();
    let mut cmd = env_command(home, &feed);
    let out = cmd
        .env("MIXLAYER_BASE_URL", "http://127.0.0.1:9/v1")
        .env("HTTPS_PROXY", "http://127.0.0.1:9")
        .args(["models", "sync", "--agent", "codex", "--dry-run"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(fs::read_dir(home).unwrap().count(), before);
    assert!(!home.join(".config").exists());
    assert!(!home.join(".codex/config.toml").exists());
}
#[test]
fn auth_token_piped_prints_raw_key() {
    let temp = tempfile::tempdir().unwrap();
    let feed = Feed::new();
    let out = env_command(temp.path(), &feed)
        .env("MIXLAYER_API_KEY", TEST_KEY)
        .args(["auth", "token", "--agent"])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(out.stdout, format!("{TEST_KEY}\n").as_bytes());
}
#[test]
fn key_catalog_filters_feed_and_adds_missing_model() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path();
    let feed = Feed::new();
    fs::create_dir_all(home.join(".config/opencode")).unwrap();
    let out = env_command(home, &feed)
        .env("MIXLAYER_API_KEY", TEST_KEY)
        .args(["models", "sync", "--agent", "opencode", "--yes"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let doc: Value = serde_json::from_slice(&fs::read(home.join(".config/opencode/opencode.json")).unwrap()).unwrap();
    let models = &doc["provider"]["mixlayer"]["models"];
    assert!(models["a/model"].is_object());
    assert!(models["missing/model"].is_object());
    assert_eq!(models["missing/model"]["limit"]["context"], 131072);
    assert_eq!(models["missing/model"]["limit"]["output"], 8192);
    assert_eq!(models.as_object().unwrap().len(), 2);
    inspect_no_key(&home.join(".config"));
}
#[test]
fn catalog_fallback_and_total_failure_leave_configs_safe() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path();
    let feed = Feed::new();
    fs::create_dir_all(home.join(".codex")).unwrap();
    let out = env_command(home, &feed)
        .env("MIXLAYER_API_KEY", TEST_KEY)
        .env("MIXLAYER_BASE_URL", "http://127.0.0.1:9/v1")
        .args(["models", "sync", "--agent", "codex", "--yes"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(home.join(".codex/config.toml").exists());
    let other = tempfile::tempdir().unwrap();
    fs::create_dir_all(other.path().join(".codex")).unwrap();
    let failed = env_command(other.path(), &feed)
        .env("MIXLAYER_API_KEY", TEST_KEY)
        .env("MIXLAYER_BASE_URL", "http://127.0.0.1:9/v1")
        .env("MIXLAYER_PLATFORM_URL", "http://127.0.0.1:9")
        .args(["models", "sync", "--agent", "codex", "--yes"])
        .output()
        .unwrap();
    assert!(!failed.status.success());
    assert!(!other.path().join(".codex/config.toml").exists());
    assert!(!other.path().join(".config/mixlayer/agent-setup.json").exists());
}
#[cfg(unix)]
#[test]
fn hermes_cli_registers_updates_and_uninstalls() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path();
    let feed = Feed::new();
    let bins = home.join("bin");
    fs::create_dir_all(&bins).unwrap();
    fs::create_dir_all(home.join(".hermes")).unwrap();
    let script = bins.join("hermes");
    fs::write(&script,"#!/bin/sh\ncase \"$1 $2\" in\n  '--version '*) echo Hermes ;;\n  'config set') printf '%s' \"$4\" > \"$HERMES_HOME/provider.json\"; printf 'set\\n' >> \"$HERMES_HOME/calls\" ;;\n  'config get') cat \"$HERMES_HOME/provider.json\" ;;\n  'config unset') rm -f \"$HERMES_HOME/provider.json\"; printf 'unset\\n' >> \"$HERMES_HOME/calls\" ;;\n  *) exit 1 ;;\nesac\n").unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
    let invoke = |args: &[&str]| {
        let out = env_command(home, &feed)
            .env(
                "PATH",
                format!("{}:{}", bins.display(), std::env::var("PATH").unwrap_or_default()),
            )
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        serde_json::from_slice::<Value>(&out.stdout).unwrap()
    };
    let args = ["models", "sync", "--agent", "hermes", "--yes"];
    assert_eq!(status(&invoke(&args)), "registered");
    assert_eq!(status(&invoke(&args)), "unchanged");
    feed.add();
    assert_eq!(invoke(&args)["actions"][0]["added"], json!(["b/model"]));
    let path = home.join(".hermes/provider.json");
    let mut doc: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    doc["name"] = "Edited".into();
    fs::write(&path, doc.to_string()).unwrap();
    assert_eq!(status(&invoke(&args)), "modified_skip");
    assert_eq!(
        status(&invoke(&[
            "models",
            "sync",
            "--agent",
            "hermes",
            "--uninstall",
            "--force",
            "--yes"
        ])),
        "removed"
    );
    assert!(!path.exists());
    assert_eq!(
        fs::read_to_string(home.join(".hermes/calls")).unwrap(),
        "set\nset\nunset\n"
    );
    inspect_no_key(home);
    // A provider the user configured themselves is never claimed or removed.
    let own = json!({"name":"Mine","api":"https://example.invalid/v1","models":{}});
    fs::write(&path, own.to_string()).unwrap();
    assert_eq!(
        status(&invoke(&["models", "sync", "--agent", "hermes"])),
        "conflict_skip"
    );
    assert_eq!(status(&invoke(&args)), "conflict_skip");
    assert_eq!(
        status(&invoke(&["models", "sync", "--agent", "hermes", "--uninstall"])),
        "not_managed"
    );
    assert_eq!(
        status(&invoke(&[
            "models",
            "sync",
            "--agent",
            "hermes",
            "--uninstall",
            "--yes"
        ])),
        "not_managed"
    );
    let kept: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(kept, own);
    assert_eq!(
        fs::read_to_string(home.join(".hermes/calls")).unwrap(),
        "set\nset\nunset\n"
    );
}
#[test]
fn jsonc_is_left_alone_with_a_pasteable_provider() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path();
    let feed = Feed::new();
    fs::create_dir_all(home.join(".config/opencode")).unwrap();
    let path = home.join(".config/opencode/opencode.json");
    let original = "{\n  // user's comment\n  \"provider\": {}\n}\n";
    fs::write(&path, original).unwrap();
    let report = run(home, &feed, &["models", "sync", "--agent", "opencode", "--yes"]);
    assert_eq!(status(&report), "conflict_skip");
    assert!(
        report["actions"][0]["note"]
            .as_str()
            .unwrap()
            .contains("@ai-sdk/openai-compatible")
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), original);
    assert!(!home.join(".config/mixlayer/agent-setup.json").exists());
}
#[test]
fn agent_setup_models_keeps_registration_state() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path();
    let feed = Feed::new();
    let mcp = home.join("mxl-mcp");
    fs::write(&mcp, "fake").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&mcp, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let args = ["agent", "setup", "--agent", "codex", "--models", "--yes"];
    let out = env_command(home, &feed)
        .env("MIXLAYER_MCP_PATH", &mcp)
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let report: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(report["models"]["actions"][0]["status"], "registered");
    let path = home.join(".config/mixlayer/agent-setup.json");
    let state: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert!(state["codex"]["models"]["installed"].as_bool().unwrap());
    let rerun = env_command(home, &feed)
        .env("MIXLAYER_MCP_PATH", &mcp)
        .args(["agent", "setup", "--agent", "codex", "--yes"])
        .output()
        .unwrap();
    assert!(rerun.status.success(), "{}", String::from_utf8_lossy(&rerun.stderr));
    let state: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert!(state["codex"]["models"]["installed"].as_bool().unwrap());
    inspect_no_key(home);
}
#[test]
fn doctor_reports_registered_models_from_cache() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path();
    let feed = Feed::new();
    fs::create_dir_all(home.join(".codex")).unwrap();
    run(home, &feed, &["models", "sync", "--agent", "codex", "--yes"]);
    let report = run(home, &feed, &["doctor", "--json"]);
    let codex = report["harnesses"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["agent"] == "codex")
        .unwrap();
    assert_eq!(codex["registered"], true);
    assert_eq!(codex["model_count"], 1);
    assert_eq!(codex["differs_from_cached_feed"], false);
}
