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
        .env("EXPECTED_MINT_KEY", NEW_SECRET)
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

fn billing_org() -> (&'static str, &'static str, u16, Value) {
    (
        "GET",
        "/v1/organizations",
        200,
        json!({"data": [{
            "id": "org_test", "name": "Test", "role": "owner", "is_personal": false,
            "billing_account_id": "ba_test"
        }]}),
    )
}

fn spend_controls() -> (&'static str, &'static str, u16, Value) {
    (
        "GET",
        "/v1/billing_accounts/ba_test/spend_controls",
        200,
        json!({"data": {
            "billing_account_id": "ba_test",
            "usage_spend_cutoff": {"threshold_cents": 2000, "local_state": "ok"},
            "usage_spend_warning": {"threshold_cents": 1500},
            "prepaid_low_balance_warning": {"threshold_cents": 500}
        }}),
    )
}

#[test]
#[cfg(unix)]
fn mint_runs_child_with_secret_then_deletes_and_preserves_status() {
    let home = tempfile::tempdir().unwrap();
    let fake = Fake::start(&[
        one_org(),
        (
            "POST",
            "/v1/organizations/org_test/api_keys",
            200,
            json!({"data": {"id": "mint_1", "key": NEW_SECRET}}),
        ),
        ("DELETE", "/v1/organizations/org_test/api_keys/mint_1", 204, Value::Null),
    ]);
    let before = chrono::Utc::now();
    let out = run(
        home.path(),
        &fake,
        &[
            "keys",
            "mint",
            "--ttl",
            "30m",
            "--permissions",
            "inference,api-read",
            "--key-env",
            "JOB_KEY",
            "--",
            "sh",
            "-c",
            "test \"$JOB_KEY\" = \"$EXPECTED_MINT_KEY\" || exit 42; printf child-ok; exit 3",
        ],
        Some(KEY),
    );
    assert_eq!(out.status.code(), Some(3), "{}", text(&out.stderr));
    assert_eq!(text(&out.stdout), "child-ok");
    assert!(!text(&out.stderr).contains(NEW_SECRET));
    assert!(text(&out.stderr).contains("Minted API key mint_1"));
    assert!(text(&out.stderr).contains("Deleted API key mint_1"));
    let seen = fake.seen();
    assert_eq!(
        seen.iter().map(|s| s.method.as_str()).collect::<Vec<_>>(),
        ["GET", "POST", "DELETE"]
    );
    let body: Value = serde_json::from_str(&seen[1].body).unwrap();
    assert_eq!(body["permissions"], json!(["inference", "api-read"]));
    let expires = chrono::DateTime::parse_from_rfc3339(body["expires_at"].as_str().unwrap()).unwrap();
    assert!(expires >= before + chrono::Duration::minutes(29));
    assert!(expires <= chrono::Utc::now() + chrono::Duration::minutes(31));

    let out = run(
        home.path(),
        &fake,
        &[
            "keys",
            "mint",
            "--quiet",
            "--",
            "sh",
            "-c",
            "test \"$MIXLAYER_API_KEY\" = \"$EXPECTED_MINT_KEY\" && printf replaced",
        ],
        Some(KEY),
    );
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(text(&out.stdout), "replaced");
    assert!(out.stderr.is_empty());
}

#[test]
#[cfg(unix)]
fn mint_deletion_failure_and_spawn_failure_still_clean_up() {
    let home = tempfile::tempdir().unwrap();
    let fake = Fake::start(&[
        one_org(),
        (
            "POST",
            "/v1/organizations/org_test/api_keys",
            200,
            json!({"data": {"id": "mint_2", "key": NEW_SECRET}}),
        ),
        (
            "DELETE",
            "/v1/organizations/org_test/api_keys/mint_2",
            500,
            json!({"error": {"message": "failed"}}),
        ),
    ]);
    let out = run(
        home.path(),
        &fake,
        &["keys", "mint", "--", "sh", "-c", "printf child-ok"],
        Some(KEY),
    );
    assert_eq!(out.status.code(), Some(5));
    assert_eq!(text(&out.stdout), "child-ok");
    assert!(text(&out.stderr).contains("mixlayer keys delete mint_2 --yes"));

    let fake = Fake::start(&[
        one_org(),
        (
            "POST",
            "/v1/organizations/org_test/api_keys",
            200,
            json!({"data": {"id": "mint_2", "key": NEW_SECRET}}),
        ),
        ("DELETE", "/v1/organizations/org_test/api_keys/mint_2", 204, Value::Null),
    ]);
    let out = run(
        home.path(),
        &fake,
        &["keys", "mint", "--", "this-command-does-not-exist-5531"],
        Some(KEY),
    );
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(fake.seen().last().unwrap().method, "DELETE");
}

#[test]
#[cfg(unix)]
fn mint_deletes_after_sigint_reaches_child_process_group() {
    use std::os::unix::process::CommandExt;

    let home = tempfile::tempdir().unwrap();
    let fake = Fake::start(&[
        one_org(),
        (
            "POST",
            "/v1/organizations/org_test/api_keys",
            200,
            json!({"data": {"id": "mint_signal", "key": NEW_SECRET}}),
        ),
        (
            "DELETE",
            "/v1/organizations/org_test/api_keys/mint_signal",
            204,
            Value::Null,
        ),
    ]);
    let mut child = Command::new(env!("CARGO_BIN_EXE_mixlayer"))
        .args(["keys", "mint", "--", "sh", "-c", "printf 'ready\\n'; exec sleep 30"])
        .env_clear()
        .env("HOME", home.path())
        .env("MIXLAYER_HOME", home.path().join("mixlayer"))
        .env("MIXLAYER_PLATFORM_URL", &fake.url)
        .env("MIXLAYER_API_KEY", KEY)
        .env("MIXLAYER_NO_AGENT_DETECT", "1")
        .process_group(0)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    assert_eq!(line, "ready\n");
    unsafe {
        libc::kill(-(child.id() as libc::pid_t), libc::SIGINT);
    }
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(130), "{}", text(&output.stderr));
    assert_eq!(fake.seen().last().unwrap().method, "DELETE");
}

/// `docker stop`, `kill <pid>` and some CI runners signal only the parent.
/// It must pass SIGTERM to the child and still delete the key.
#[test]
#[cfg(unix)]
fn mint_forwards_sigterm_sent_only_to_the_parent() {
    use std::time::{Duration, Instant};

    let home = tempfile::tempdir().unwrap();
    let fake = Fake::start(&[
        one_org(),
        (
            "POST",
            "/v1/organizations/org_test/api_keys",
            200,
            json!({"data": {"id": "mint_term", "key": NEW_SECRET}}),
        ),
        (
            "DELETE",
            "/v1/organizations/org_test/api_keys/mint_term",
            204,
            Value::Null,
        ),
    ]);
    let started = Instant::now();
    let mut child = Command::new(env!("CARGO_BIN_EXE_mixlayer"))
        .args(["keys", "mint", "--", "sh", "-c", "printf 'ready\\n'; exec sleep 30"])
        .env_clear()
        .env("HOME", home.path())
        .env("MIXLAYER_HOME", home.path().join("mixlayer"))
        .env("MIXLAYER_PLATFORM_URL", &fake.url)
        .env("MIXLAYER_API_KEY", KEY)
        .env("MIXLAYER_NO_AGENT_DETECT", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    assert_eq!(line, "ready\n");
    unsafe {
        // The parent's PID only, not the process group.
        libc::kill(child.id() as libc::pid_t, libc::SIGTERM);
    }
    let output = child.wait_with_output().unwrap();
    assert!(started.elapsed() < Duration::from_secs(20), "child was not stopped");
    assert_eq!(output.status.code(), Some(143), "{}", text(&output.stderr));
    assert_eq!(fake.seen().last().unwrap().method, "DELETE");
}

#[test]
fn mint_validates_before_sending_and_dry_run_does_not_send() {
    let home = tempfile::tempdir().unwrap();
    let fake = Fake::start(&[]);
    for args in [
        &["keys", "mint"][..],
        &["keys", "mint", "--ttl", "0", "--", "job"][..],
        &["keys", "mint", "--ttl", "25h", "--", "job"][..],
    ] {
        let out = run(home.path(), &fake, args, Some(KEY));
        assert_eq!(out.status.code(), Some(2), "{args:?}: {}", text(&out.stderr));
    }
    let out = run(
        home.path(),
        &fake,
        &["keys", "mint", "--ttl", "2h", "--dry-run", "--json", "--", "job"],
        None,
    );
    assert!(out.status.success(), "{}", text(&out.stderr));
    let plan: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(plan["create"]["method"], "POST");
    assert_eq!(plan["delete"]["method"], "DELETE");
    assert!(plan["create"]["body"]["expires_at"].is_string());
    assert!(fake.seen().is_empty());
}

#[test]
fn whoami_reports_permissions_and_handles_inference_only_keys() {
    let home = tempfile::tempdir().unwrap();
    let key = "sk_mxl_9dNa8zjc_synthetic";
    let fake = Fake::start(&[
        billing_org(),
        (
            "GET",
            "/v1/organizations/org_test/api_keys",
            200,
            json!({"data": [{
                "id": "9dNa8zjc", "name": "ci", "permissions": ["inference", "api-admin"],
                "active": true, "expires_at": null
            }]}),
        ),
    ]);
    let out = run(home.path(), &fake, &["whoami", "--json"], Some(key));
    assert!(out.status.success(), "{}", text(&out.stderr));
    let report: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(report["key_id"], "9dNa8zjc");
    assert_eq!(report["can"]["admin"], true);
    assert_eq!(report["organization"]["billing_account_id"], "ba_test");
    assert!(!text(&out.stdout).contains(key));
    assert!(!text(&out.stderr).contains(key));

    let denied = Fake::start(&[("GET", "/v1/organizations", 403, json!({"error": "forbidden"}))]);
    let out = run(home.path(), &denied, &["whoami", "--json"], Some(key));
    assert!(out.status.success());
    let report: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(report["platform_access"], "none");
    assert!(report["can"]["inference"].is_null());
    assert_eq!(report["can"]["read"], false);

    let rejected = Fake::start(&[("GET", "/v1/organizations", 401, json!({"error": "bad key"}))]);
    let out = run(home.path(), &rejected, &["whoami"], Some(key));
    assert_eq!(out.status.code(), Some(4));
    assert!(!text(&out.stderr).contains(key));
}

#[test]
fn spend_reports_current_and_recent_ranges_and_gates_after_output() {
    let home = tempfile::tempdir().unwrap();
    let fake = Fake::start(&[
        billing_org(),
        spend_controls(),
        (
            "GET",
            "/v1/billing_accounts/ba_test/usage/current",
            200,
            json!({"data": [{
                "start_date": "2026-10-01", "end_date": "2026-10-31", "subtotal_cents": 525.125,
                "total_applied_credits_cents": -100.0, "total_amount_cents": 425.125,
                "line_item_groups": [{"model": "small", "total_cents": 25.125}, {"model": "large", "total_cents": 500.0},
                    {"model": null, "total_cents": -100.0}]
            }]}),
        ),
        (
            "GET",
            "/v1/billing_accounts/ba_test/usage_spend?range=30d",
            200,
            json!({"data": {
                "range_start": "2026-09-10", "range_end": "2026-10-10", "bucket_ms": 86400000,
                "models": [{"model": "large", "total_cents": 500.0}, {"model": "small", "total_cents": 25.125}],
                "buckets": [{"date": "2026-10-09", "total_cents": 525.125}]
            }}),
        ),
    ]);
    let out = run(home.path(), &fake, &["spend", "--json"], Some(KEY));
    assert!(out.status.success(), "{}", text(&out.stderr));
    let report: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(report["usage_usd"], 5.25125);
    assert_eq!(report["credits_usd"], -1.0);
    assert_eq!(report["net_usd"], 4.25125);
    assert_eq!(report["by_model"][0]["model"], "large");
    // The credit group (no model, negative) is reported as credits_usd only.
    assert_eq!(report["by_model"].as_array().unwrap().len(), 2, "{report}");
    assert_eq!(report["limit_remaining_usd"], 14.74875);
    let human = run(home.path(), &fake, &["spend"], Some(KEY));
    assert!(
        text(&human.stdout).contains("Credits: -$1.00  Net: $4.25"),
        "{}",
        text(&human.stdout)
    );

    let out = run(home.path(), &fake, &["spend", "--range", "30d", "--json"], Some(KEY));
    assert!(out.status.success(), "{}", text(&out.stderr));
    let report: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(report["daily"][0]["usd"], 5.25125);
    assert_eq!(report["by_model"][0]["model"], "large");

    let over = run(home.path(), &fake, &["spend", "--fail-over", "5", "--json"], Some(KEY));
    assert_eq!(over.status.code(), Some(9));
    assert_eq!(
        serde_json::from_slice::<Value>(&over.stdout).unwrap()["usage_usd"],
        5.25125
    );
    assert!(text(&over.stderr).contains("spend $5.25 is over --fail-over $5.00"));
    let under = run(home.path(), &fake, &["spend", "--fail-over", "6", "--json"], Some(KEY));
    assert!(under.status.success());
}

#[test]
fn budget_show_and_set_preserve_other_controls() {
    let home = tempfile::tempdir().unwrap();
    let fake = Fake::start(&[
        billing_org(),
        spend_controls(),
        (
            "GET",
            "/v1/billing_accounts/ba_test?include=all",
            200,
            json!({"data": {"billing_mode": "prepaid", "credit_balance_cents": 1234}}),
        ),
        (
            "PUT",
            "/v1/billing_accounts/ba_test/spend_controls",
            200,
            json!({"data": {
                "usage_spend_cutoff": {"threshold_cents": 3000, "local_state": "ok"},
                "usage_spend_warning": {"threshold_cents": 1500},
                "prepaid_low_balance_warning": {"threshold_cents": 500}
            }}),
        ),
    ]);
    let out = run(home.path(), &fake, &["budget", "show", "--json"], Some(KEY));
    assert!(out.status.success(), "{}", text(&out.stderr));
    let report: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(report["monthly_limit_usd"], 20.0);
    assert_eq!(report["prepaid_balance_usd"], 12.34);

    let out = run(
        home.path(),
        &fake,
        &["budget", "set", "--monthly-limit", "30", "--yes", "--json"],
        Some(KEY),
    );
    assert!(out.status.success(), "{}", text(&out.stderr));
    let put = fake.seen().into_iter().find(|s| s.method == "PUT").unwrap();
    let body: Value = serde_json::from_str(&put.body).unwrap();
    assert_eq!(body["usage_spend_cutoff_threshold_cents"], 3000);
    assert_eq!(body["usage_spend_warning_threshold_cents"], 1500);
    assert_eq!(body["prepaid_low_balance_warning_threshold_cents"], 500);

    let out = run(
        home.path(),
        &fake,
        &["budget", "set", "--low-balance-warning", "off", "--yes"],
        Some(KEY),
    );
    assert!(out.status.success(), "{}", text(&out.stderr));
    let put = fake.seen().into_iter().rfind(|s| s.method == "PUT").unwrap();
    let body: Value = serde_json::from_str(&put.body).unwrap();
    assert!(body["prepaid_low_balance_warning_threshold_cents"].is_null());
}

#[test]
fn budget_rejects_unconfirmed_or_invalid_changes_and_dry_run_sends_no_put() {
    let home = tempfile::tempdir().unwrap();
    let fake = Fake::start(&[billing_org(), spend_controls()]);
    let out = run(
        home.path(),
        &fake,
        &["budget", "set", "--monthly-limit", "30", "--agent"],
        Some(KEY),
    );
    assert_eq!(out.status.code(), Some(2));
    assert!(fake.seen().is_empty());
    let out = run(
        home.path(),
        &fake,
        &["budget", "set", "--warn-at", "20", "--yes"],
        Some(KEY),
    );
    assert_eq!(out.status.code(), Some(2));
    assert!(!fake.seen().iter().any(|s| s.method == "PUT"));
    let out = run(
        home.path(),
        &fake,
        &["budget", "set", "--monthly-limit", "12.345", "--yes"],
        Some(KEY),
    );
    assert_eq!(out.status.code(), Some(2));
    let count = fake.seen().len();
    let out = run(
        home.path(),
        &fake,
        &["budget", "set", "--monthly-limit", "30", "--dry-run", "--json"],
        Some(KEY),
    );
    assert!(out.status.success(), "{}", text(&out.stderr));
    let plan: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(plan["request"]["body"]["usage_spend_warning_threshold_cents"], 1500);
    assert_eq!(fake.seen().len(), count + 3); // organization resolution, billing lookup and controls are read only
    assert!(!fake.seen().iter().any(|s| s.method == "PUT"));

    let out = run(
        home.path(),
        &fake,
        &["budget", "set", "--warn-at", "15", "--dry-run", "--json"],
        None,
    );
    assert!(out.status.success(), "{}", text(&out.stderr));
    let plan: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(plan["request"]["body"]["usage_spend_warning_threshold_cents"], 1500);
    assert_eq!(
        plan["request"]["body"]["usage_spend_cutoff_threshold_cents"],
        "<current>"
    );
}

fn audit_entry(id: &str, time: &str) -> Value {
    json!({"id": id, "created_at": time, "action": "api_key.create", "resource_type": "api_key",
        "resource_id": id, "actor": "key$/9dNa8zjc", "metadata": {"name": "ci", "permissions": ["inference"]}})
}

#[test]
fn audit_passes_filters_and_keeps_full_metadata_in_json() {
    let home = tempfile::tempdir().unwrap();
    let expected = "/v1/organizations/org_test/audit_log?limit=2&from=2026-10-01T00%3A00%3A00Z&action=api_key.&actor=key%24%2F9dNa8zjc&resource_type=api_key";
    let fake = Fake::start(&[(
        "GET",
        expected,
        200,
        json!({"data": [audit_entry("a1", "2026-10-10T00:00:00Z")],
        "start_cursor": "s1", "end_cursor": "e1"}),
    )]);
    let out = run(
        home.path(),
        &fake,
        &[
            "audit",
            "--org",
            "org_test",
            "--limit",
            "2",
            "--since",
            "2026-10-01T00:00:00Z",
            "--action",
            "api_key.",
            "--actor",
            "key$/9dNa8zjc",
            "--resource-type",
            "api_key",
            "--json",
        ],
        Some(KEY),
    );
    assert!(out.status.success(), "{}", text(&out.stderr));
    let report: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(report["data"][0]["metadata"]["permissions"], json!(["inference"]));
    assert_eq!(fake.seen()[0].path, expected);
}

#[test]
#[cfg(unix)]
fn audit_follow_uses_before_cursor_and_stops_on_sigint() {
    use std::sync::mpsc;
    use std::time::Duration;

    let home = tempfile::tempdir().unwrap();
    let initial = "/v1/organizations/org_test/audit_log?limit=50";
    let newer = "/v1/organizations/org_test/audit_log?limit=50&before=s1";
    let fake = Fake::start(&[
        (
            "GET",
            initial,
            200,
            json!({"data": [audit_entry("a1", "2026-10-10T00:00:00Z")],
            "start_cursor": "s1", "end_cursor": "e1"}),
        ),
        (
            "GET",
            newer,
            200,
            json!({"data": [audit_entry("a2", "2026-10-10T00:01:00Z")],
            "start_cursor": "s2", "end_cursor": "e2"}),
        ),
    ]);
    let mut child = Command::new(env!("CARGO_BIN_EXE_mixlayer"))
        .args(["audit", "--org", "org_test", "--follow", "--interval", "1s", "--json"])
        .env_clear()
        .env("HOME", home.path())
        .env("MIXLAYER_HOME", home.path().join("mixlayer"))
        .env("MIXLAYER_PLATFORM_URL", &fake.url)
        .env("MIXLAYER_API_KEY", KEY)
        .env("MIXLAYER_NO_AGENT_DETECT", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let _ = tx.send(line.unwrap());
        }
    });
    let first = rx.recv_timeout(Duration::from_secs(5));
    let second = rx.recv_timeout(Duration::from_secs(5));
    unsafe {
        libc::kill(child.id() as libc::pid_t, libc::SIGINT);
    }
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(serde_json::from_str::<Value>(&first.unwrap()).unwrap()["id"], "a1");
    assert_eq!(serde_json::from_str::<Value>(&second.unwrap()).unwrap()["id"], "a2");
    assert!(fake.seen().iter().any(|s| s.path == newer));
}

/// With nothing in the first page there is no cursor; follow must keep
/// polling the plain query instead of waiting forever.
#[test]
#[cfg(unix)]
fn audit_follow_keeps_polling_when_the_first_page_is_empty() {
    use std::time::Duration;

    let home = tempfile::tempdir().unwrap();
    let initial = "/v1/organizations/org_test/audit_log?limit=50";
    let fake = Fake::start(&[(
        "GET",
        initial,
        200,
        json!({"data": [], "start_cursor": null, "end_cursor": null}),
    )]);
    let child = Command::new(env!("CARGO_BIN_EXE_mixlayer"))
        .args(["audit", "--org", "org_test", "--follow", "--interval", "1s", "--json"])
        .env_clear()
        .env("HOME", home.path())
        .env("MIXLAYER_HOME", home.path().join("mixlayer"))
        .env("MIXLAYER_PLATFORM_URL", &fake.url)
        .env("MIXLAYER_API_KEY", KEY)
        .env("MIXLAYER_NO_AGENT_DETECT", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    thread::sleep(Duration::from_millis(2600));
    unsafe {
        libc::kill(child.id() as libc::pid_t, libc::SIGINT);
    }
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{}", text(&output.stderr));
    let polls = fake.seen().iter().filter(|s| s.path == initial).count();
    assert!(polls >= 2, "only {polls} request(s); follow stopped polling");
}
