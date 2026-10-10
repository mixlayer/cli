use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Command, Output};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};

struct Request {
    path: String,
    body: Value,
    auth: String,
}
struct FakeServer {
    url: String,
    requests: Arc<Mutex<Vec<Request>>>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}
impl FakeServer {
    fn start() -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let url = format!("http://{}", listener.local_addr()?);
        listener.set_nonblocking(true)?;
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let reqs = requests.clone();
        let flag = stop.clone();
        let thread = thread::spawn(move || {
            while !flag.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let reqs = reqs.clone();
                        thread::spawn(move || handle(stream, reqs));
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => thread::sleep(Duration::from_millis(5)),
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            url,
            requests,
            stop,
            thread: Some(thread),
        })
    }
    fn seen(&self) -> Vec<(String, Value)> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .map(|r| (r.path.clone(), r.body.clone()))
            .collect()
    }
    fn auths(&self) -> Vec<String> {
        self.requests.lock().unwrap().iter().map(|r| r.auth.clone()).collect()
    }
}
impl Drop for FakeServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.thread.take() {
            handle.join().unwrap();
        }
    }
}
fn handle(mut stream: TcpStream, requests: Arc<Mutex<Vec<Request>>>) {
    // Accepted sockets inherit the listener's non-blocking mode on macOS and
    // BSD; without this, a read before the request arrives fails and the
    // server drops the connection unanswered.
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut data = Vec::new();
    let mut buf = [0u8; 4096];
    let mut end = None;
    loop {
        match stream.read(&mut buf) {
            Ok(0) | Err(_) => return,
            Ok(n) => {
                data.extend_from_slice(&buf[..n]);
                if end.is_none() {
                    end = data.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4);
                }
                if let Some(i) = end {
                    let header = String::from_utf8_lossy(&data[..i]);
                    let len = header
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .and_then(|v| v.trim().parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    if data.len() >= i + len {
                        break;
                    }
                }
            }
        }
    }
    let i = end.unwrap();
    let header = String::from_utf8_lossy(&data[..i]);
    let path = header
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .unwrap_or("")
        .to_owned();
    let body = serde_json::from_slice(&data[i..]).unwrap_or(Value::Null);
    let auth = header
        .lines()
        .find_map(|line| {
            line.split_once(':').and_then(|(name, value)| {
                name.eq_ignore_ascii_case("authorization")
                    .then(|| value.trim().to_owned())
            })
        })
        .unwrap_or_default();
    requests.lock().unwrap().push(Request {
        path: path.clone(),
        body: body.clone(),
        auth,
    });
    let (status, response) = reply(&path, &body);
    let bytes = response.to_string();
    let msg = if status == 200 { "OK" } else { "Error" };
    let text = format!(
        "HTTP/1.1 {status} {msg}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{bytes}",
        bytes.len()
    );
    let _ = stream.write_all(text.as_bytes());
}
fn reply(path: &str, body: &Value) -> (u16, Value) {
    if path == "/v1/models" {
        return (
            200,
            json!({"object":"list", "data":[{"id":"fake/model","object":"model"}]}),
        );
    }
    if path == "/v1/organizations" {
        return (200, json!({"data":[{"id":"org"}]}));
    }
    if path == "/v1/organizations/org/models" {
        return (
            200,
            json!({"data":[
                {"model_sku":"cheap","status":"active","price_m_input_tokens_usd":1.0,"price_m_output_tokens_usd":2.0,"price_m_input_cache_read_tokens_usd":0.5},
                {"model_sku":"mid","status":"active","price_m_input_tokens_usd":2.0,"price_m_output_tokens_usd":4.0,"price_m_input_cache_read_tokens_usd":1.0},
                {"model_sku":"frontier","status":"active","price_m_input_tokens_usd":5.0,"price_m_output_tokens_usd":15.0,"price_m_input_cache_read_tokens_usd":2.0},
                {"model_sku":"broken","status":"active","price_m_input_tokens_usd":3.0,"price_m_output_tokens_usd":5.0,"price_m_input_cache_read_tokens_usd":1.0}
            ]}),
        );
    }
    if path == "/v1/systemone" {
        let model = body["model"].as_str().unwrap_or("fixture");
        let state = body["state"].as_str().unwrap_or("");
        let status = match state {
            "auth" => 401,
            "billing" => 402,
            "forbidden" => 403,
            "bad" | "invalid" => 422,
            "rate" => 429,
            "capacity" => 529,
            "server" => 500,
            _ => 200,
        };
        if status != 200 {
            return (status, json!({"error":{"code":"fixture_error","message":"rejected"}}));
        }
        if state == "malformed" {
            return (200, json!({}));
        }
        return (
            200,
            json!({"model":model,"answers":{"q":{"type":"noul","noul":0.8}},"usage":{"input_tokens":12,"output_tokens":0}}),
        );
    }
    if path == "/v1/chat/completions" {
        let model = body["model"].as_str().unwrap_or("");
        if model == "broken" {
            return (500, json!({"error":{"message":"failed"}}));
        }
        let question = body["messages"][0]["content"].as_str().unwrap_or("");
        let answer = if question.contains("CONFIDENCE:") {
            if model == "cheap" {
                "uncertain. CONFIDENCE: low"
            } else {
                "Answer. CONFIDENCE: high"
            }
        } else if question.contains("Synthesize these model answers") {
            "Synthesis"
        } else {
            "Answer"
        };
        return (
            200,
            json!({"choices":[{"message":{"content":answer,"reasoning_content":"trace"}}],"usage":{"prompt_tokens":1000,"completion_tokens":1000,"total_tokens":2000}}),
        );
    }
    (404, json!({"error":"unknown path"}))
}
fn cli(home: &std::path::Path, base: &str, args: &[&str]) -> Output {
    cli_env(home, base, args, &[])
}
fn cli_env(home: &std::path::Path, base: &str, args: &[&str], extra: &[(&str, &str)]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_mixlayer"));
    command
        .args(args)
        .env_clear()
        // Windows networking needs SYSTEMROOT (WSA error 10106 without it).
        .envs(std::env::vars().filter(|(k, _)| matches!(k.to_ascii_uppercase().as_str(), "SYSTEMROOT" | "WINDIR")))
        .env("HOME", home)
        .env("MIXLAYER_HOME", home.join("mixlayer"))
        .env("MIXLAYER_BASE_URL", format!("{base}/v1"))
        .env("MIXLAYER_PLATFORM_URL", base)
        .env("MIXLAYER_API_KEY", "mlk-test-dummy")
        .env("MIXLAYER_NO_AGENT_DETECT", "1")
        .env("MIXLAYER_ENABLE_JINI", "1");
    for (key, value) in extra {
        command.env(key, value);
    }
    command.output().unwrap()
}
fn value(out: &Output) -> Value {
    serde_json::from_slice(&out.stdout).unwrap_or_else(|_| {
        panic!(
            "stdout: {} stderr: {}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    })
}
#[test]
fn dry_runs_and_sql_gate_match_go() {
    let home = tempfile::tempdir().unwrap();
    let base = "http://127.0.0.1:9";
    for (args, key) in [
        (
            &["ladder", "question", "--rungs", "cheap,mid", "--dry-run", "--json"][..],
            "models",
        ),
        (
            &[
                "compare",
                "question",
                "--cheap",
                "cheap",
                "--frontier",
                "frontier",
                "--dry-run",
                "--json",
            ][..],
            "models",
        ),
        (
            &[
                "council",
                "question",
                "--members",
                "cheap,mid",
                "--judge",
                "frontier",
                "--dry-run",
                "--json",
            ][..],
            "models",
        ),
    ] {
        let out = cli(home.path(), base, args);
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        assert!(value(&out)[key].is_array());
    }
    for args in [
        ["sql", "delete from runs", "--agent"],
        ["sql", "select value from vault", "--agent"],
    ] {
        let out = cli(home.path(), base, &args);
        assert_eq!(out.status.code(), Some(2));
    }
    let preview = cli(
        home.path(),
        base,
        &[
            "decide",
            "--model",
            "fixture",
            "--state-text",
            "facts",
            "--noul",
            "q=?",
            "--dry-run",
            "--agent",
        ],
    );
    assert!(preview.status.success());
    assert_eq!(value(&preview)["body"]["state"], "facts");
    let state = home.path().join("state.json");
    std::fs::write(&state, r#"{"order_id":9007199254740993}"#).unwrap();
    let preview = cli(
        home.path(),
        base,
        &[
            "decide",
            "--model",
            "fixture",
            "--state",
            state.to_str().unwrap(),
            "--noul",
            "q=?",
            "--dry-run",
            "--agent",
        ],
    );
    assert_eq!(value(&preview)["body"]["state"]["order_id"], 9007199254740993u64);
    for extra in [
        vec!["--noul", "q=?", "--noul", "q=again"],
        vec!["--noul", "q=?", "--fail-if", "missing>1"],
        vec!["--noul", "q=?", "--fail-if", "q>=NaN"],
        vec!["--noul", "q=?", "--fail-if", "q==yes"],
    ] {
        let mut args = vec!["decide", "--model", "fixture", "--state-text", "facts"];
        args.extend(extra);
        let out = cli(home.path(), base, &args);
        assert_eq!(out.status.code(), Some(2), "{}", String::from_utf8_lossy(&out.stderr));
    }
    let rows = home.path().join("rows.csv");
    let questions = home.path().join("questions.json");
    let dest = home.path().join("results.jsonl");
    std::fs::write(&rows, "text,id\nfirst,1\nsecond,2\nthird,3\n").unwrap();
    std::fs::write(&questions, r#"{"q":{"type":"noul","instructions":"?"}}"#).unwrap();
    let preview = cli(
        home.path(),
        base,
        &[
            "decide",
            "batch",
            "--model",
            "fixture",
            "--input",
            rows.to_str().unwrap(),
            "--questions",
            questions.to_str().unwrap(),
            "--max-rows",
            "2",
            "--out",
            dest.to_str().unwrap(),
            "--dry-run",
            "--json",
        ],
    );
    assert!(preview.status.success(), "{}", String::from_utf8_lossy(&preview.stderr));
    assert_eq!(value(&preview)["rows"], 2);
    assert!(!dest.exists());
    let unsafe_out = cli(
        home.path(),
        base,
        &[
            "decide",
            "batch",
            "--model",
            "fixture",
            "--input",
            rows.to_str().unwrap(),
            "--questions",
            questions.to_str().unwrap(),
            "--out",
            rows.to_str().unwrap(),
        ],
    );
    assert_eq!(unsafe_out.status.code(), Some(2));
    let alias = home.path().join("rows-alias.csv");
    std::fs::hard_link(&rows, &alias).unwrap();
    let unsafe_out = cli(
        home.path(),
        base,
        &[
            "decide",
            "batch",
            "--model",
            "fixture",
            "--input",
            rows.to_str().unwrap(),
            "--questions",
            questions.to_str().unwrap(),
            "--out",
            alias.to_str().unwrap(),
        ],
    );
    assert_eq!(unsafe_out.status.code(), Some(2));
    assert_eq!(
        std::fs::read_to_string(&rows).unwrap(),
        "text,id\nfirst,1\nsecond,2\nthird,3\n"
    );
}
#[test]
fn local_ledger_search_replay_savings_and_sql() {
    let home = tempfile::tempdir().unwrap();
    let base = "http://127.0.0.1:9";
    let db = home.path().join("ledger.db");
    let init = cli(home.path(), base, &["savings", "--db", db.to_str().unwrap(), "--json"]);
    assert!(init.status.success());
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute("INSERT INTO runs (id,command,prompt,answer,reasoning,model,seed,prompt_tokens,completion_tokens,total_tokens,cost_usd,latency_ms,created_at) VALUES (?1,'ask','migration plan','use a phased rollout','trace','cheap',7,1000,1000,2000,0.003,450,'2026-10-09T00:00:00Z')",["run_fixture"]).unwrap();
    conn.execute("INSERT INTO runs_fts (id,prompt,answer,reasoning) VALUES ('run_fixture','migration plan','use a phased rollout','trace')",[]).unwrap();
    drop(conn);
    let grep = cli(
        home.path(),
        base,
        &["grep", "migration", "--db", db.to_str().unwrap(), "--json"],
    );
    assert!(grep.status.success());
    assert_eq!(value(&grep).as_array().unwrap().len(), 1);
    let replay = cli(
        home.path(),
        base,
        &[
            "replay",
            "run_fixture",
            "--db",
            db.to_str().unwrap(),
            "--dry-run",
            "--json",
        ],
    );
    assert!(replay.status.success());
    let v = value(&replay);
    assert_eq!(v["models"][0], "cheap");
    assert_eq!(v["seed"], 7);
    assert_eq!(v["prompt_summary"]["bytes"], 14);
    let savings = cli(home.path(), base, &["savings", "--db", db.to_str().unwrap(), "--json"]);
    let v = value(&savings);
    assert_eq!(v["runs"], 1);
    assert_eq!(v["tokens"], 2000);
    assert_eq!(v["baseline_usd"], 0.02);
    assert_eq!(v["saved_usd"], 0.017);
    let sql = cli(
        home.path(),
        base,
        &[
            "sql",
            "select count(*) as n from runs",
            "--db",
            db.to_str().unwrap(),
            "--agent",
        ],
    );
    assert!(sql.status.success());
    assert_eq!(value(&sql)[0]["n"], 1);
    let private = cli(
        home.path(),
        base,
        &[
            "sql",
            "select value from vault",
            "--db",
            db.to_str().unwrap(),
            "--agent",
        ],
    );
    assert_eq!(private.status.code(), Some(2));
    // Ledger commands honour the output formats (Go printed JSON for all three).
    let d = db.to_str().unwrap();
    let text = |args: &[&str]| {
        let out = cli(home.path(), base, args);
        assert!(out.status.success(), "{args:?}");
        String::from_utf8(out.stdout).unwrap()
    };
    let q = "select id, prompt, cost_usd from runs";
    assert_eq!(
        text(&["sql", q, "--db", d, "--csv"]),
        "id,prompt,cost_usd\nrun_fixture,migration plan,0.003\n"
    );
    assert_eq!(
        text(&["sql", q, "--db", d, "--plain"]),
        "id\tprompt\tcost_usd\nrun_fixture\tmigration plan\t0.003\n"
    );
    assert_eq!(text(&["grep", "migration", "--db", d, "--quiet"]), "run_fixture\n");
    // An explicit format beats agent mode; an explicit --json beats the format.
    assert!(text(&["sql", q, "--db", d, "--agent", "--csv"]).starts_with("id,prompt,cost_usd\n"));
    assert_eq!(
        value(&cli(home.path(), base, &["sql", q, "--db", d, "--csv", "--json"]))[0]["id"],
        "run_fixture"
    );
    assert!(text(&["savings", "--db", d, "--csv"]).starts_with("runs,excluded_unknown_cost_runs,tokens,"));
}
#[test]
fn model_refresh_and_live_catalog_cache_survive_separate_cli_runs() {
    let Ok(server) = FakeServer::start() else {
        eprintln!("loopback bind denied; model cache HTTP test unverified");
        return;
    };
    let home = tempfile::tempdir().unwrap();
    let db = home.path().join("custom.db");
    let db = db.to_str().unwrap();
    let refresh = cli(
        home.path(),
        &server.url,
        &["models", "query", "fake", "--refresh", "--db", db],
    );
    assert!(refresh.status.success(), "{}", String::from_utf8_lossy(&refresh.stderr));
    let cached = cli(home.path(), &server.url, &["models", "query", "fake", "--db", db]);
    assert_eq!(value(&cached), value(&refresh));
    assert_eq!(server.seen().iter().filter(|(p, _)| p == "/v1/models").count(), 1);
    let first = cli(
        home.path(),
        &server.url,
        &["models", "list", "--live-prices", "--json", "--db", db],
    );
    assert!(first.status.success());
    let second = cli(
        home.path(),
        &server.url,
        &["models", "list", "--live-prices", "--json", "--db", db],
    );
    assert_eq!(value(&first), value(&second));
    let ask = cli(home.path(), &server.url, &["ask", "question", "--json", "--db", db]);
    assert!(ask.status.success(), "{}", String::from_utf8_lossy(&ask.stderr));
    assert_eq!(value(&ask)["model"], "frontier");
    assert_eq!(
        server
            .seen()
            .iter()
            .filter(|(p, _)| p == "/v1/organizations/org/models")
            .count(),
        1
    );
}

#[test]
fn chat_machine_flags_work_with_valid_messages() {
    let Ok(server) = FakeServer::start() else {
        eprintln!("loopback bind denied; chat output HTTP test unverified");
        return;
    };
    let home = tempfile::tempdir().unwrap();
    let args = [
        "chat",
        "--model",
        "cheap",
        "--messages",
        r#"[{"role":"user","content":"hi"}]"#,
    ];
    let out = cli(home.path(), &server.url, &args);
    assert!(out.status.success());
    assert_eq!(value(&out)["meta"]["source"], "live");
    assert!(value(&out)["results"]["choices"].is_array());
    let mut quiet = args.to_vec();
    quiet.push("--quiet");
    let out = cli(home.path(), &server.url, &quiet);
    assert!(out.status.success());
    // --quiet prints just the reply text.
    let reply = String::from_utf8(out.stdout).unwrap();
    assert!(!reply.trim().is_empty() && !reply.contains('{'), "{reply}");
    assert!(out.stderr.is_empty());
    let mut csv = args.to_vec();
    csv.push("--csv");
    let out = cli(home.path(), &server.url, &csv);
    assert!(out.status.success());
    let csv = String::from_utf8(out.stdout).unwrap();
    assert_eq!(csv.lines().count(), 2, "header plus one row: {csv}");
    assert!(csv.lines().next().unwrap().split(',').any(|c| c == "choices"), "{csv}");
}

#[test]
fn ladder_escalate_council_and_ledger() {
    let Ok(server) = FakeServer::start() else {
        eprintln!("loopback bind denied; fake HTTP test unverified");
        return;
    };
    let home = tempfile::tempdir().unwrap();
    let out = cli(
        home.path(),
        &server.url,
        &["ladder", "Question", "--rungs", "cheap,broken", "--json"],
    );
    assert_eq!(out.status.code(), Some(6), "{}", String::from_utf8_lossy(&out.stderr));
    let v = value(&out);
    assert_eq!(v["results"].as_array().unwrap().len(), 2);
    assert!(v["results"][1]["cost_usd"].is_null());
    let out = cli(
        home.path(),
        &server.url,
        &["escalate", "Question", "--rungs", "cheap,mid,frontier", "--json"],
    );
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let v = value(&out);
    assert_eq!(v["results"].as_array().unwrap().len(), 2);
    assert_eq!(v["results"][1]["checker_score"], 0.9);
    assert_eq!(v["saved_usd"], 0.011);
    let out = cli(
        home.path(),
        &server.url,
        &[
            "council",
            "Question",
            "--members",
            "cheap,mid",
            "--judge",
            "frontier",
            "--json",
        ],
    );
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let v = value(&out);
    assert_eq!(v["members"].as_array().unwrap().len(), 2);
    assert_eq!(v["judge"][0]["answer"], "Synthesis");
    let seen = server.seen();
    assert!(seen.iter().any(|(path, body)| {
        path == "/v1/chat/completions"
            && body["messages"][0]["content"]
                .as_str()
                .unwrap_or("")
                .contains("---BEGIN MEMBER 1---")
    }));
    let query = cli(
        home.path(),
        &server.url,
        &["sql", "select command, group_id from runs", "--json"],
    );
    assert!(query.status.success());
    assert!(
        value(&query)
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["command"] == "council-judge")
    );
}
#[test]
fn compare_replay_grep_and_savings() {
    let Ok(server) = FakeServer::start() else {
        eprintln!("loopback bind denied; fake HTTP test unverified");
        return;
    };
    let home = tempfile::tempdir().unwrap();
    let ask = cli(
        home.path(),
        &server.url,
        &["ask", "Question", "--model", "cheap", "--seed", "7", "--json"],
    );
    assert!(ask.status.success(), "{}", String::from_utf8_lossy(&ask.stderr));
    let run = value(&ask)["id"].as_str().unwrap().to_owned();
    let replay = cli(home.path(), &server.url, &["replay", &run, "--json"]);
    assert!(replay.status.success(), "{}", String::from_utf8_lossy(&replay.stderr));
    assert_eq!(value(&replay)["same_answer"], true);
    let compare = cli(
        home.path(),
        &server.url,
        &[
            "compare",
            "Question",
            "--cheap",
            "cheap",
            "--frontier",
            "frontier",
            "--json",
        ],
    );
    assert!(compare.status.success(), "{}", String::from_utf8_lossy(&compare.stderr));
    assert_eq!(value(&compare)["delta_usd"], 0.017);
    let grep = cli(home.path(), &server.url, &["grep", "Question", "--json"]);
    assert!(grep.status.success());
    assert!(value(&grep).as_array().unwrap().len() >= 4);
    let savings = cli(home.path(), &server.url, &["savings", "--json"]);
    assert!(savings.status.success());
    assert_eq!(value(&savings)["runs"], 4);
    let seen = server.seen();
    assert!(
        seen.iter()
            .filter(|(path, body)| path == "/v1/chat/completions" && body["model"] == "cheap" && body["seed"] == 7)
            .count()
            >= 2
    );
}
#[test]
fn decide_gates_and_batch_rows() {
    let Ok(server) = FakeServer::start() else {
        eprintln!("loopback bind denied; fake HTTP test unverified");
        return;
    };
    let home = tempfile::tempdir().unwrap();
    let out = cli(
        home.path(),
        &server.url,
        &[
            "decide",
            "--model",
            "fixture",
            "--state-text",
            "facts",
            "--noul",
            "q=?",
            "--fail-if",
            "q>0.7",
            "--json",
        ],
    );
    assert_eq!(out.status.code(), Some(9));
    assert_eq!(value(&out)["answers"]["q"]["noul"], 0.8);
    let rows = home.path().join("rows.jsonl");
    let questions = home.path().join("questions.json");
    std::fs::write(&rows, "{\"text\":\"good\"}\n{\"text\":\"bad\"}\n").unwrap();
    std::fs::write(&questions, r#"{"q":{"type":"noul","instructions":"?"}}"#).unwrap();
    let out = cli(
        home.path(),
        &server.url,
        &[
            "decide",
            "batch",
            "--model",
            "fixture",
            "--input",
            rows.to_str().unwrap(),
            "--questions",
            questions.to_str().unwrap(),
            "--state-field",
            "text",
            "--concurrency",
            "2",
        ],
    );
    assert_eq!(out.status.code(), Some(6), "{}", String::from_utf8_lossy(&out.stderr));
    let lines: Vec<Value> = out
        .stdout
        .split(|b| *b == b'\n')
        .filter(|l| !l.is_empty())
        .map(|l| serde_json::from_slice(l).unwrap())
        .collect();
    assert_eq!(lines.len(), 2);
    assert!(lines.iter().any(|r| r["row"] == 2 && r["error"]["code"] == 2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("rows ok=1 failed=1"));
}
#[test]
fn decide_http_statuses_and_response_contract() {
    let Ok(server) = FakeServer::start() else {
        eprintln!("loopback bind denied; fake HTTP test unverified");
        return;
    };
    let home = tempfile::tempdir().unwrap();
    for (state, code) in [
        ("auth", 4),
        ("forbidden", 4),
        ("billing", 8),
        ("invalid", 2),
        ("rate", 7),
        ("capacity", 5),
        ("server", 5),
        ("malformed", 5),
    ] {
        let out = cli(
            home.path(),
            &server.url,
            &[
                "decide",
                "--model",
                "fixture",
                "--state-text",
                state,
                "--noul",
                "q=?",
                "--json",
            ],
        );
        assert_eq!(
            out.status.code(),
            Some(code),
            "{state}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        if state != "malformed" {
            assert!(String::from_utf8_lossy(&out.stderr).contains("hint:"));
        }
    }
    let seen = server.seen();
    for state in ["rate", "capacity"] {
        assert_eq!(
            seen.iter()
                .filter(|(path, body)| path == "/v1/systemone" && body["state"] == state)
                .count(),
            4
        );
    }
}
#[test]
fn decide_foreign_endpoint_credentials() {
    let Ok(mixlayer) = FakeServer::start() else {
        eprintln!("loopback bind denied; fake HTTP test unverified");
        return;
    };
    let foreign = FakeServer::start().unwrap();
    let home = tempfile::tempdir().unwrap();
    let target = format!("{}/v1", foreign.url);
    let args = [
        "decide",
        "--model",
        "fixture",
        "--state-text",
        "facts",
        "--noul",
        "q=?",
        "--endpoint",
        target.as_str(),
        "--json",
    ];
    let out = cli(home.path(), &mixlayer.url, &args);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(foreign.auths().last().unwrap(), "");
    let mut keyed = args.to_vec();
    keyed.extend(["--endpoint-key-env", "SELFHOST_KEY"]);
    let out = cli_env(
        home.path(),
        &mixlayer.url,
        &keyed,
        &[("SELFHOST_KEY", "local-test-key")],
    );
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(foreign.auths().last().unwrap(), "Bearer local-test-key");
    let same = format!("{}/v1", mixlayer.url);
    let out = cli(
        home.path(),
        &mixlayer.url,
        &[
            "decide",
            "--model",
            "fixture",
            "--state-text",
            "facts",
            "--noul",
            "q=?",
            "--endpoint",
            same.as_str(),
            "--json",
        ],
    );
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(mixlayer.auths().last().unwrap(), "Bearer mlk-test-dummy");
}
