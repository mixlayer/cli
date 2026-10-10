// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

//! Platform API commands (`keys`) and `agent setup --platform`, against a
//! local fake Platform API. Keys here are synthetic.

use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;

use serde_json::{Value, json};

const KEY: &str = "mxl_synthetic_platform_key_5531";
const NEW_SECRET: &str = "mxl_ak_new_SYNTHETICSECRET_0042";

#[derive(Clone, Debug)]
struct Seen {
    method: String,
    path: String,
    auth: String,
    body: String,
}

/// Answers `(METHOD, path)` with a canned status and body and records every request.
struct Fake {
    url: String,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl Fake {
    fn start(routes: &[(&str, &str, u16, Value)]) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let routes: Arc<HashMap<(String, String), (u16, String)>> = Arc::new(
            routes
                .iter()
                .map(|(m, p, s, b)| {
                    let body = if b.is_null() { String::new() } else { b.to_string() };
                    (((*m).to_owned(), (*p).to_owned()), (*s, body))
                })
                .collect(),
        );
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let routes = routes.clone();
                let log = log.clone();
                thread::spawn(move || handle(stream, &routes, &log));
            }
        });
        Self { url, seen }
    }

    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}

fn handle(stream: TcpStream, routes: &HashMap<(String, String), (u16, String)>, log: &Mutex<Vec<Seen>>) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() || line.is_empty() {
        return;
    }
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_owned();
    let path = parts.next().unwrap_or_default().to_owned();
    let (mut auth, mut length) = (String::new(), 0usize);
    loop {
        let mut header = String::new();
        reader.read_line(&mut header).unwrap();
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            match name.to_ascii_lowercase().as_str() {
                "authorization" => auth = value.trim().to_owned(),
                "content-length" => length = value.trim().parse().unwrap_or(0),
                _ => {}
            }
        }
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body).unwrap();
    log.lock().unwrap().push(Seen {
        method: method.clone(),
        path: path.clone(),
        auth,
        body: String::from_utf8_lossy(&body).into_owned(),
    });
    let (status, text) = routes
        .get(&(method, path))
        .cloned()
        .unwrap_or((404, r#"{"error":{"message":"not found"}}"#.into()));
    let mut out = stream;
    let _ = write!(
        out,
        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
        text.len()
    );
}

fn one_org() -> (&'static str, &'static str, u16, Value) {
    (
        "GET",
        "/v1/organizations",
        200,
        json!({"data": [{"id": "org_test", "name": "Test"}]}),
    )
}

fn run(home: &Path, fake: &Fake, args: &[&str], key: Option<&str>) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_mixlayer"));
    cmd.args(args)
        .env_clear()
        .envs(std::env::vars().filter(|(k, _)| matches!(k.to_ascii_uppercase().as_str(), "SYSTEMROOT" | "WINDIR")))
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("MIXLAYER_HOME", home.join("mixlayer"))
        .env("MIXLAYER_PLATFORM_URL", &fake.url)
        .env("MIXLAYER_BASE_URL", "http://127.0.0.1:9/v1")
        .env("MIXLAYER_NO_AGENT_DETECT", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(key) = key {
        cmd.env("MIXLAYER_API_KEY", key);
    }
    cmd.output().unwrap()
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

#[test]
fn list_resolves_the_only_org_and_never_shows_secrets() {
    let home = tempfile::tempdir().unwrap();
    let fake = Fake::start(&[
        one_org(),
        (
            "GET",
            "/v1/organizations/org_test/api_keys",
            200,
            json!({"data": [{"id": "ak_1", "name": "laptop", "active": true, "permissions": ["inference"], "created_at": "2026-10-01T00:00:00Z", "expires_at": null}]}),
        ),
    ]);
    let out = run(home.path(), &fake, &["keys", "list", "--json"], Some(KEY));
    assert!(out.status.success(), "{}", text(&out.stderr));
    let listed: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(listed["results"][0]["id"], "ak_1", "{listed}");
    let seen = fake.seen();
    assert_eq!(
        seen.iter().map(|s| s.path.as_str()).collect::<Vec<_>>(),
        ["/v1/organizations", "/v1/organizations/org_test/api_keys"]
    );
    assert!(seen.iter().all(|s| s.auth == format!("Bearer {KEY}")));
}

#[test]
fn create_sends_permissions_and_prints_the_secret_alone_on_stdout() {
    let home = tempfile::tempdir().unwrap();
    let fake = Fake::start(&[
        one_org(),
        (
            "POST",
            "/v1/organizations/org_test/api_keys",
            200,
            json!({"data": {"id": "ak_new", "name": "claude", "active": true, "permissions": ["inference", "api-read"], "created_at": "2026-10-09T00:00:00Z", "expires_at": null, "key": NEW_SECRET}}),
        ),
    ]);
    let out = run(
        home.path(),
        &fake,
        &[
            "keys",
            "create",
            "--name",
            "claude",
            "--permissions",
            "inference,api-read,api-read",
        ],
        Some(KEY),
    );
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(text(&out.stdout), format!("{NEW_SECRET}\n"));
    assert!(text(&out.stderr).contains("only time the secret is shown"));
    let post = fake.seen().into_iter().find(|s| s.method == "POST").unwrap();
    let body: Value = serde_json::from_str(&post.body).unwrap();
    assert_eq!(
        body,
        json!({"name": "claude", "permissions": ["inference", "api-read"]})
    );
}

#[test]
fn create_save_writes_credentials_and_prints_no_secret() {
    let home = tempfile::tempdir().unwrap();
    let fake = Fake::start(&[(
        "POST",
        "/v1/organizations/org_x/api_keys",
        200,
        json!({"data": {"id": "ak_new", "name": "", "active": true, "permissions": ["inference"], "created_at": "2026-10-09T00:00:00Z", "expires_at": null, "key": NEW_SECRET}}),
    )]);
    let out = run(
        home.path(),
        &fake,
        &["keys", "create", "--org", "org_x", "--save", "--json"],
        Some(KEY),
    );
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert!(!text(&out.stdout).contains(NEW_SECRET));
    assert!(!text(&out.stderr).contains(NEW_SECRET));
    let shown: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(shown.get("key").is_none());
    let creds = fs::read_to_string(home.path().join("mixlayer/data/credentials.toml")).unwrap();
    assert!(creds.contains(NEW_SECRET));
    // --org skips the organization lookup.
    assert_eq!(fake.seen().len(), 1);
}

#[test]
fn bad_permission_and_dry_run_send_nothing() {
    let home = tempfile::tempdir().unwrap();
    let fake = Fake::start(&[one_org()]);
    let out = run(
        home.path(),
        &fake,
        &["keys", "create", "--permissions", "root"],
        Some(KEY),
    );
    assert_eq!(out.status.code(), Some(2), "{}", text(&out.stderr));
    let out = run(
        home.path(),
        &fake,
        &["keys", "create", "--permissions", "api-read", "--dry-run", "--json"],
        Some(KEY),
    );
    assert!(out.status.success(), "{}", text(&out.stderr));
    let plan: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(plan["method"], "POST");
    assert_eq!(plan["url"], format!("{}/v1/organizations/<org-id>/api_keys", fake.url));
    assert_eq!(plan["body"]["permissions"], json!(["api-read"]));
    assert!(fake.seen().is_empty());
}

#[test]
fn rotate_and_delete_need_an_explicit_yes() {
    let home = tempfile::tempdir().unwrap();
    let fake = Fake::start(&[
        one_org(),
        (
            "POST",
            "/v1/organizations/org_test/api_keys/ak_1/rotate",
            200,
            json!({"data": {"id": "ak_1", "name": "laptop", "active": true, "permissions": ["inference"], "created_at": "2026-10-01T00:00:00Z", "expires_at": null, "key": NEW_SECRET}}),
        ),
        ("DELETE", "/v1/organizations/org_test/api_keys/ak_1", 204, Value::Null),
    ]);
    // No terminal and no --yes; --agent implies --yes but must not count.
    for args in [
        &["keys", "rotate", "ak_1"][..],
        &["keys", "delete", "ak_1", "--agent"][..],
    ] {
        let out = run(home.path(), &fake, args, Some(KEY));
        assert_eq!(out.status.code(), Some(2), "{args:?}: {}", text(&out.stderr));
        assert!(text(&out.stderr).contains("--yes"));
    }
    assert!(fake.seen().is_empty());

    let out = run(
        home.path(),
        &fake,
        &["keys", "rotate", "ak_1", "--yes", "--json"],
        Some(KEY),
    );
    assert!(out.status.success(), "{}", text(&out.stderr));
    let rotated: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(rotated["key"], NEW_SECRET);
    let out = run(
        home.path(),
        &fake,
        &["keys", "delete", "ak_1", "--yes", "--json"],
        Some(KEY),
    );
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(serde_json::from_slice::<Value>(&out.stdout).unwrap()["deleted"], true);
    let methods: Vec<_> = fake
        .seen()
        .into_iter()
        .filter(|s| s.method != "GET")
        .map(|s| (s.method, s.path))
        .collect();
    assert_eq!(
        methods,
        [
            ("POST".into(), "/v1/organizations/org_test/api_keys/ak_1/rotate".into()),
            ("DELETE".into(), "/v1/organizations/org_test/api_keys/ak_1".into()),
        ]
    );
}

#[test]
fn several_orgs_require_org_and_403_names_the_permission() {
    let home = tempfile::tempdir().unwrap();
    let fake = Fake::start(&[(
        "GET",
        "/v1/organizations",
        200,
        json!({"data": [{"id": "org_a"}, {"id": "org_b"}]}),
    )]);
    let out = run(home.path(), &fake, &["keys", "list"], Some(KEY));
    assert_eq!(out.status.code(), Some(2));
    assert!(text(&out.stderr).contains("org_a, org_b"), "{}", text(&out.stderr));

    let fake = Fake::start(&[(
        "GET",
        "/v1/organizations",
        403,
        json!({"error": {"message": "insufficient API key permissions"}}),
    )]);
    let out = run(home.path(), &fake, &["keys", "list"], Some(KEY));
    assert_eq!(out.status.code(), Some(4));
    let err = text(&out.stderr);
    assert!(err.contains("api-read permission"), "{err}");
    assert!(!err.contains(KEY));

    let out = run(home.path(), &fake, &["keys", "list"], None);
    assert_eq!(out.status.code(), Some(4));
}

// ---- agent setup --platform

fn executable(path: &Path, body: &str) {
    fs::write(path, body).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }
}

fn setup(home: &Path, fake: &Fake, args: &[&str], key: Option<&str>, path_dir: Option<&Path>) -> Output {
    let bins = home.join("bin");
    fs::create_dir_all(&bins).unwrap();
    let mcp = bins.join("mixlayer-mcp");
    executable(&mcp, "#!/bin/sh\nexit 0\n");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_mixlayer"));
    cmd.args(["agent", "setup"])
        .args(args)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("MIXLAYER_MCP_PATH", &mcp)
        .env("CLAUDE_CONFIG_DIR", home.join(".claude"))
        .env("MIXLAYER_PLATFORM_URL", &fake.url)
        .env("MIXLAYER_NO_AGENT_DETECT", "1")
        .env_remove("MIXLAYER_API_KEY")
        .env_remove("MIXLAYER_PLATFORM_KEY");
    let path = std::env::var("PATH").unwrap_or_default();
    match path_dir {
        Some(dir) => cmd.env("PATH", format!("{}:{path}", dir.display())),
        // Keep a real `claude` on this machine out of the test.
        None => cmd.env("PATH", "/usr/bin:/bin"),
    };
    if let Some(key) = key {
        cmd.env("MIXLAYER_PLATFORM_KEY", key);
    }
    cmd.output().unwrap()
}

fn report(out: &Output) -> Value {
    assert!(out.status.success(), "{}", text(&out.stderr));
    serde_json::from_slice(&out.stdout).unwrap()
}

fn action<'a>(report: &'a Value, agent: &str, kind: &str) -> &'a Value {
    report["actions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["agent"] == agent && a["kind"] == kind)
        .unwrap_or_else(|| panic!("no {agent}/{kind} action in {report}"))
}

fn assert_no_key(dir: &Path) {
    for item in fs::read_dir(dir).unwrap().flatten() {
        let path = item.path();
        if path.is_dir() {
            assert_no_key(&path);
        } else {
            assert!(
                !fs::read_to_string(&path).unwrap_or_default().contains(KEY),
                "{}",
                path.display()
            );
        }
    }
}

#[test]
fn platform_setup_writes_env_references_for_codex_and_cursor() {
    let home = tempfile::tempdir().unwrap();
    let fake = Fake::start(&[one_org()]);
    let codex = home.path().join(".codex/config.toml");
    fs::create_dir_all(codex.parent().unwrap()).unwrap();
    let original = b"# mine\n[other]\nmode = 'keep'\n";
    fs::write(&codex, original).unwrap();
    for agent in ["codex", "cursor"] {
        let out = setup(
            home.path(),
            &fake,
            &[
                "--agent",
                agent,
                "--platform",
                "--key-env",
                "MIXLAYER_PLATFORM_KEY",
                "--yes",
            ],
            Some(KEY),
            None,
        );
        let r = report(&out);
        assert_eq!(action(&r, agent, "platform-mcp")["status"], "registered", "{r}");
        assert_eq!(action(&r, "platform", "platform-key")["status"], "ok", "{r}");
    }
    let toml: toml::Value = toml::from_str(&fs::read_to_string(&codex).unwrap()).unwrap();
    let entry = &toml["mcp_servers"]["mixlayer-platform"];
    assert_eq!(entry["url"].as_str(), Some(format!("{}/v1/mcp", fake.url).as_str()));
    assert_eq!(entry["bearer_token_env_var"].as_str(), Some("MIXLAYER_PLATFORM_KEY"));
    let cursor: Value = serde_json::from_slice(&fs::read(home.path().join(".cursor/mcp.json")).unwrap()).unwrap();
    assert_eq!(
        cursor["mcpServers"]["mixlayer-platform"],
        json!({"url": format!("{}/v1/mcp", fake.url), "headers": {"Authorization": "Bearer ${env:MIXLAYER_PLATFORM_KEY}"}})
    );
    // The local server is still installed alongside.
    assert!(cursor["mcpServers"]["mixlayer"]["command"].is_string());
    assert_no_key(home.path());

    // Uninstall without --platform still removes what setup added.
    for agent in ["codex", "cursor"] {
        let r = report(&setup(
            home.path(),
            &fake,
            &["--agent", agent, "--uninstall", "--yes"],
            None,
            None,
        ));
        assert_eq!(action(&r, agent, "platform-mcp")["status"], "removed", "{r}");
    }
    assert_eq!(fs::read(&codex).unwrap(), original);
    // Setup created mcp.json, so removing its last entry removes the file.
    assert!(!home.path().join(".cursor/mcp.json").exists());
}

#[test]
#[cfg(unix)] // drives a fake `claude` shell script
fn platform_setup_registers_claude_code_over_http_with_env_header() {
    let home = tempfile::tempdir().unwrap();
    let fake = Fake::start(&[one_org()]);
    let bins = tempfile::tempdir().unwrap();
    let args_log = home.path().join("claude-args.txt");
    // Records each invocation's arguments, one per line, and writes the
    // entry `claude mcp add` would.
    executable(
        &bins.path().join("claude"),
        &format!(
            "#!/bin/sh\nfor a in \"$@\"; do printf '%s\\n' \"$a\"; done >> '{}'\nprintf -- '---\\n' >> '{}'\nconfig=\"$CLAUDE_CONFIG_DIR/.claude.json\"\ncase \"$*\" in\n  *'mcp add'*mixlayer-platform*) printf '{{\"mcpServers\":{{\"mixlayer\":{{\"command\":\"x\"}},\"mixlayer-platform\":{{\"type\":\"http\"}}}}}}\\n' > \"$config\" ;;\n  *'mcp add'*) printf '{{\"mcpServers\":{{\"mixlayer\":{{\"command\":\"x\"}}}}}}\\n' > \"$config\" ;;\n  *) printf '{{}}\\n' > \"$config\" ;;\nesac\n",
            args_log.display(),
            args_log.display()
        ),
    );
    fs::create_dir_all(home.path().join(".claude")).unwrap();
    fs::write(home.path().join(".claude/.claude.json"), "{}\n").unwrap();
    let r = report(&setup(
        home.path(),
        &fake,
        &["--agent", "claude-code", "--platform", "--yes"],
        None,
        Some(bins.path()),
    ));
    let platform = action(&r, "claude-code", "platform-mcp");
    assert_eq!(platform["status"], "registered", "{r}");
    assert_eq!(action(&r, "platform", "platform-key")["status"], "missing", "{r}");
    let calls = fs::read_to_string(&args_log).unwrap();
    let add = calls.split("---\n").find(|c| c.contains("mixlayer-platform")).unwrap();
    let args: Vec<&str> = add.lines().collect();
    assert_eq!(
        args,
        [
            "mcp",
            "add",
            "--transport",
            "http",
            "--scope",
            "user",
            "mixlayer-platform",
            &format!("{}/v1/mcp", fake.url),
            "--header",
            "Authorization: Bearer ${MIXLAYER_API_KEY}",
        ]
    );
    // The command shown to the user quotes the header for a POSIX shell.
    assert!(
        platform["command"]
            .as_str()
            .unwrap()
            .ends_with("--header 'Authorization: Bearer ${MIXLAYER_API_KEY}'")
    );
}

#[test]
fn platform_setup_reports_key_problems_and_unsupported_agents() {
    let home = tempfile::tempdir().unwrap();
    let fake = Fake::start(&[(
        "GET",
        "/v1/organizations",
        403,
        json!({"error": {"message": "insufficient API key permissions"}}),
    )]);
    let r = report(&setup(
        home.path(),
        &fake,
        &[
            "--agent",
            "claude-desktop",
            "--platform",
            "--key-env",
            "MIXLAYER_PLATFORM_KEY",
        ],
        Some(KEY),
        None,
    ));
    assert_eq!(r["dry_run"], true);
    let check = action(&r, "platform", "platform-key");
    assert_eq!(check["status"], "no_platform_permission");
    assert!(check["note"].as_str().unwrap().contains("api-read"));
    assert_eq!(action(&r, "claude-desktop", "platform-mcp")["status"], "unsupported");
    assert_eq!(fake.seen()[0].auth, format!("Bearer {KEY}"));
    // Preview writes nothing.
    assert!(!home.path().join(".config/mixlayer/agent-setup.json").exists());

    let out = setup(
        home.path(),
        &fake,
        &["--agent", "codex", "--platform", "--key-env", "BAD-NAME"],
        None,
        None,
    );
    assert_eq!(out.status.code(), Some(2), "{}", text(&out.stderr));
}
