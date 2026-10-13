use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};

fn cli(home: &Path, args: &[&str], input: Option<&str>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_mxl"));
    command
        .args(args)
        .env_clear()
        // Windows networking needs SYSTEMROOT (WSA error 10106 without it).
        .envs(std::env::vars().filter(|(k, _)| matches!(k.to_ascii_uppercase().as_str(), "SYSTEMROOT" | "WINDIR")))
        .env("HOME", home)
        .env("MIXLAYER_HOME", home.join("mixlayer"))
        .env("MIXLAYER_NO_AGENT_DETECT", "1")
        .env("TEST_FRONTIER_KEY", "dummy")
        .env("TEST_DETECTOR_KEY", "dummy");
    command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    if input.is_some() {
        command.stdin(std::process::Stdio::piped());
    }
    let mut child = command.spawn().unwrap();
    if let Some(input) = input {
        child.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
    }
    child.wait_with_output().unwrap()
}
fn val(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&output.stdout)))
}
fn body(mut stream: &std::net::TcpStream) -> Value {
    stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let mut bytes = Vec::new();
    let mut buf = [0; 8192];
    loop {
        let n = stream.read(&mut buf).unwrap();
        if n == 0 {
            break;
        }
        bytes.extend_from_slice(&buf[..n]);
        if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
            let header = String::from_utf8_lossy(&bytes[..end]);
            let len = header
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .and_then(|v| v.trim().parse::<usize>().ok())
                })
                .unwrap_or(0);
            if bytes.len() >= end + 4 + len {
                let mut request: Value = serde_json::from_slice(&bytes[end + 4..end + 4 + len]).unwrap();
                request["_test_path"] = header.lines().next().unwrap_or("").into();
                request["_test_authorization"] = header
                    .lines()
                    .find_map(|line| {
                        line.split_once(':')
                            .filter(|(name, _)| name.eq_ignore_ascii_case("authorization"))
                            .map(|(_, value)| value.trim())
                    })
                    .unwrap_or("")
                    .into();
                return request;
            }
        }
    }
    panic!("incomplete request")
}
fn fake_server<F>(count: usize, mut reply: F) -> Option<(String, mpsc::Receiver<Value>, thread::JoinHandle<()>)>
where
    F: FnMut(&Value) -> Value + Send + 'static,
{
    let listener = match TcpListener::bind("127.0.0.1:0") {
        Ok(listener) => listener,
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            eprintln!("fake HTTP test unverified: loopback binding denied by sandbox");
            return None;
        }
        Err(e) => panic!("loopback binding failed: {e}"),
    };
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (tx, rx) = mpsc::channel();
    let worker = thread::spawn(move || {
        for _ in 0..count {
            let (mut stream, _) = listener.accept().unwrap();
            let request = body(&stream);
            let response = reply(&request);
            let _ = tx.send(request); // the test may not read requests
            let text = response.to_string();
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}", text.len()).unwrap();
        }
    });
    Some((url, rx, worker))
}
fn chat(answer: &str) -> Value {
    json!({"choices":[{"message":{"content":answer}}]})
}
fn base_args<'a>(base: &'a str, db: &'a str) -> Vec<&'a str> {
    vec![
        "shield",
        "ask",
        "Email Zora Quill at zora@fiction.example or call 312-555-0199",
        "--upstream",
        "custom",
        "--accept-regex-only",
        "--upstream-base-url",
        base,
        "--upstream-key-env",
        "TEST_FRONTIER_KEY",
        "--model",
        "fake",
        "--db",
        db,
        "--json",
    ]
}

#[test]
fn shield_ask_round_trip_receipt_and_dry_run() {
    let home = tempfile::tempdir().unwrap();
    let db = home.path().join("vault.db");
    let db = db.to_str().unwrap();
    let Some((url, rx, worker)) = fake_server(1, |request| {
        let text = request["messages"][1]["content"].as_str().unwrap();
        assert_eq!(request["messages"][0]["role"], "system");
        assert_eq!(request["_test_authorization"], "Bearer dummy");
        assert_eq!(request["_test_path"], "POST /v1/chat/completions HTTP/1.1");
        assert!(
            request["messages"][0]["content"]
                .as_str()
                .unwrap()
                .contains("Repeat each placeholder exactly")
        );
        for raw in ["Zora Quill", "zora@fiction.example", "312-555-0199"] {
            assert!(!request.to_string().contains(raw), "{text}");
        }
        let tokens = regex::Regex::new(r"[A-Z_]+_[0-9a-f]{8}")
            .unwrap()
            .find_iter(text)
            .map(|m| m.as_str())
            .collect::<Vec<_>>();
        chat(&format!("Reply to {}", tokens.join(" and ")))
    }) else {
        return;
    };
    let base = format!("{url}/v1");
    let args = base_args(&base, db);
    let mut dry = args.clone();
    dry.push("--dry-run");
    let preview = cli(home.path(), &dry, None);
    assert!(preview.status.success(), "{}", String::from_utf8_lossy(&preview.stderr));
    let preview = val(&preview);
    assert_eq!(preview["dry_run"], true);
    assert_eq!(preview["tripwire_hits"], json!([]));
    assert_eq!(preview["messages"][1]["content"], preview["payload"]);
    assert!(!preview["payload"].as_str().unwrap().contains("zora@fiction.example"));
    assert!(rx.try_recv().is_err());
    let output = cli(home.path(), &args, None);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let result = val(&output);
    assert!(result["answer"].as_str().unwrap().contains("zora@fiction.example"));
    assert!(result["answer"].as_str().unwrap().contains("Zora Quill"));
    assert!(result["answer"].as_str().unwrap().contains("312-555-0199"));
    assert_eq!(result["audit"]["tripwire_detector"], "regex");
    assert_eq!(result["audit"]["restored_count"], 3);
    assert!(result["audit"]["cost_usd"].is_null());
    assert!(!result["run"]["prompt"].as_str().unwrap().contains("Zora Quill"));
    assert!(rx.recv_timeout(Duration::from_secs(2)).is_ok());
    worker.join().unwrap();
    let rows = cli(home.path(), &["shield", "audit", "--db", db, "--json"], None);
    assert!(rows.status.success());
    assert_eq!(val(&rows).as_array().unwrap().len(), 1);
    let conn = rusqlite::Connection::open(db).unwrap();
    let (stored_prompt, stored_cost): (String, Option<f64>) = conn
        .query_row(
            "SELECT prompt,cost_usd FROM runs WHERE command='shield ask'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert!(stored_prompt.contains("PERSON_") && stored_prompt.contains("EMAIL_") && stored_prompt.contains("PHONE_"));
    for raw in ["Zora Quill", "zora@fiction.example", "312-555-0199"] {
        assert!(!stored_prompt.contains(raw));
    }
    assert_eq!(stored_cost, None);
    let vault = cli(home.path(), &["vault", "list", "--db", db, "--json"], None);
    let vault_text = String::from_utf8(vault.stdout).unwrap();
    assert!(!vault_text.contains("zora@fiction.example"));
    assert!(!vault_text.contains("Zora Quill"));
}

#[test]
fn ask_near_miss_only_restores_request_tokens() {
    let home = tempfile::tempdir().unwrap();
    let db = home.path().join("vault.db");
    let db = db.to_str().unwrap();
    let Some((url, _, worker)) = fake_server(1, |request| {
        let text = request["messages"][1]["content"].as_str().unwrap();
        let token = regex::Regex::new(r"EMAIL_[0-9a-f]{8}")
            .unwrap()
            .find(text)
            .unwrap()
            .as_str();
        json!({"choices":[{"message":{"content":format!("{} EMAIL_deadbeef", token.to_lowercase()),"reasoning_content":format!("Saw {token}")}}]})
    }) else {
        return;
    };
    let base = format!("{url}/v1");
    let output = cli(
        home.path(),
        &[
            "shield",
            "ask",
            "Email zora@fiction.example",
            "--upstream",
            "custom",
            "--accept-regex-only",
            "--upstream-base-url",
            &base,
            "--upstream-key-env",
            "TEST_FRONTIER_KEY",
            "--model",
            "fake",
            "--db",
            db,
            "--json",
        ],
        None,
    );
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let result = val(&output);
    assert!(result["answer"].as_str().unwrap().contains("zora@fiction.example"));
    assert!(result["answer"].as_str().unwrap().contains("EMAIL_deadbeef"));
    assert_eq!(result["restored_count"], 1);
    assert_eq!(result["unrestored_count"], 1);
    assert_eq!(result["audit"]["repairs"].as_array().unwrap().len(), 1);
    assert_eq!(result["run"]["reasoning"], "Saw zora@fiction.example");
    worker.join().unwrap();
}

#[test]
fn shield_local_commands_and_exit_codes() {
    let home = tempfile::tempdir().unwrap();
    let db = home.path().join("vault.db");
    let db = db.to_str().unwrap();
    let data = home.path().join("input.txt");
    fs::write(&data, "Zora Quill: zora@fiction.example").unwrap();
    let data = data.to_str().unwrap();
    let scan = cli(home.path(), &["shield", "scan", data, "--db", db, "--json"], None);
    assert_eq!(scan.status.code(), Some(6));
    assert_eq!(val(&scan)["risk"], 9);
    let help = cli(home.path(), &["shield", "scan", "--help"], None);
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("per-entity max risk"));
    let scan = cli(
        home.path(),
        &["shield", "scan", data, "--db", db, "--max-risk", "9", "--json"],
        None,
    );
    assert!(scan.status.success());
    let redacted = cli(
        home.path(),
        &["shield", "redact", data, "--db", db, "--diff", "--json"],
        None,
    );
    assert!(redacted.status.success());
    let masked = val(&redacted);
    assert!(masked["text"].as_str().unwrap().contains("EMAIL_"));
    assert!(String::from_utf8_lossy(&redacted.stdout).contains("zora@fiction.example"));
    let hidden = cli(home.path(), &["shield", "redact", data, "--db", db, "--json"], None);
    assert!(!String::from_utf8_lossy(&hidden.stdout).contains("zora@fiction.example"));
    let rehydrated = cli(
        home.path(),
        &["vault", "rehydrate", masked["text"].as_str().unwrap(), "--db", db],
        None,
    );
    assert!(String::from_utf8_lossy(&rehydrated.stdout).contains("zora@fiction.example"));
    let rotate = cli(home.path(), &["vault", "rotate", "--json"], None);
    assert_eq!(val(&rotate)["rotated"], false);
    let purge = cli(home.path(), &["vault", "purge", "--db", db, "--json"], None);
    assert_eq!(purge.status.code(), Some(2));
    let purge = cli(home.path(), &["vault", "purge", "--db", db, "--yes", "--json"], None);
    assert!(purge.status.success());
    let list = cli(home.path(), &["vault", "list", "--db", db, "--json"], None);
    assert_eq!(val(&list), json!([]));
}

#[test]
fn dictionary_columns_restructure_ingest_and_eval() {
    let home = tempfile::tempdir().unwrap();
    let db = home.path().join("vault.db");
    let db = db.to_str().unwrap();
    let added = cli(
        home.path(),
        &[
            "shield",
            "dict",
            "add",
            "moon pebble",
            "--kind",
            "PERSON",
            "--db",
            db,
            "--json",
        ],
        None,
    );
    assert_eq!(val(&added)["kind"], "PERSON");
    let listed = cli(home.path(), &["shield", "dict", "list", "--db", db, "--json"], None);
    assert_eq!(val(&listed)[0]["term"], "moon pebble");
    let removed = cli(
        home.path(),
        &["shield", "dict", "remove", "moon pebble", "--db", db, "--json"],
        None,
    );
    assert_eq!(val(&removed)["removed"], true);
    let data = home.path().join("customers.csv");
    fs::write(
        &data,
        "customer,company,email,age\nZora Quill,Fieldstone Labs,zora@fiction.example,42\n",
    )
    .unwrap();
    let data = data.to_str().unwrap();
    let redacted = cli(
        home.path(),
        &[
            "shield",
            "redact",
            data,
            "--mask-columns",
            "customer,company",
            "--db",
            db,
            "--json",
        ],
        None,
    );
    assert!(
        redacted.status.success(),
        "{}",
        String::from_utf8_lossy(&redacted.stderr)
    );
    let masked = val(&redacted)["text"].as_str().unwrap().to_string();
    assert!(!masked.contains("Fieldstone Labs"));
    assert!(!masked.contains("Zora Quill"));
    assert!(masked.contains("COMPANY_"));
    let preview = cli(
        home.path(),
        &[
            "shield",
            "ask",
            "Which accounts at Fieldstone Labs might churn?",
            "--data",
            data,
            "--mask-columns",
            "customer,company",
            "--upstream",
            "custom",
            "--accept-regex-only",
            "--upstream-base-url",
            "http://127.0.0.1:9/v1",
            "--upstream-key-env",
            "TEST_FRONTIER_KEY",
            "--model",
            "fake",
            "--db",
            db,
            "--dry-run",
            "--json",
        ],
        None,
    );
    assert!(preview.status.success(), "{}", String::from_utf8_lossy(&preview.stderr));
    let payload = val(&preview)["payload"].as_str().unwrap().to_string();
    assert!(!payload.contains("Fieldstone Labs"));
    assert!(!payload.contains("Zora Quill"));
    assert!(payload.contains("COMPANY_"));
    let restructured = cli(
        home.path(),
        &["shield", "restructure", data, "--bucket", "--drop", "email"],
        None,
    );
    let out = String::from_utf8(restructured.stdout).unwrap();
    assert!(out.contains("10-99"));
    assert!(!out.contains("zora@fiction.example"));
    let ingested = cli(
        home.path(),
        &["shield", "ingest", data, "--db", db, "--max-bytes", "100", "--json"],
        None,
    );
    assert!(ingested.status.success());
    assert!(val(&ingested)["masked_corpus"].as_str().unwrap().contains("EMAIL_"));
    let ingested_human = cli(home.path(), &["shield", "ingest", data, "--db", db], None);
    assert!(ingested_human.status.success());
    assert!(String::from_utf8_lossy(&ingested_human.stdout).contains("EMAIL_"));
    let eval = cli(
        home.path(),
        &[
            "shield",
            "eval",
            "--sample",
            "testdata/shield-eval/synthetic.jsonl",
            "--min-recall",
            "0",
            "--json",
        ],
        None,
    );
    assert!(eval.status.success(), "{}", String::from_utf8_lossy(&eval.stderr));
    assert_eq!(val(&eval)["results"][0]["detector"], "regex");
    let threshold = cli(
        home.path(),
        &[
            "shield",
            "eval",
            "--sample",
            "testdata/shield-eval/synthetic.jsonl",
            "--min-recall",
            "0.9",
            "--json",
        ],
        None,
    );
    assert_eq!(threshold.status.code(), Some(2));
}

#[test]
fn ask_requires_external_opt_in_model_key_and_regex_dry_run() {
    let home = tempfile::tempdir().unwrap();
    let db = home.path().join("vault.db");
    let db = db.to_str().unwrap();
    let base = "http://127.0.0.1:9/v1";
    let no_model = cli(
        home.path(),
        &[
            "shield",
            "ask",
            "hi",
            "--upstream",
            "custom",
            "--upstream-base-url",
            base,
            "--upstream-key-env",
            "TEST_FRONTIER_KEY",
            "--db",
            db,
        ],
        None,
    );
    assert_eq!(no_model.status.code(), Some(2));
    let no_opt_in = cli(
        home.path(),
        &[
            "shield",
            "ask",
            "hi",
            "--upstream",
            "custom",
            "--upstream-base-url",
            base,
            "--upstream-key-env",
            "TEST_FRONTIER_KEY",
            "--model",
            "fake",
            "--db",
            db,
        ],
        None,
    );
    assert_eq!(no_opt_in.status.code(), Some(2));
    let missing_key = cli(
        home.path(),
        &[
            "shield",
            "ask",
            "hi",
            "--upstream",
            "custom",
            "--accept-regex-only",
            "--upstream-base-url",
            base,
            "--upstream-key-env",
            "TEST_ABSENT_KEY",
            "--model",
            "fake",
            "--db",
            db,
        ],
        None,
    );
    assert_eq!(missing_key.status.code(), Some(4));
    let model_dry = cli(
        home.path(),
        &[
            "shield",
            "ask",
            "hi",
            "--upstream",
            "custom",
            "--upstream-base-url",
            base,
            "--upstream-key-env",
            "TEST_FRONTIER_KEY",
            "--model",
            "fake",
            "--detector",
            "local:fake",
            "--dry-run",
            "--db",
            db,
        ],
        None,
    );
    assert_eq!(model_dry.status.code(), Some(2));
    let invalid_local = cli(
        home.path(),
        &[
            "shield",
            "scan",
            "testdata/shield-eval/synthetic.jsonl",
            "--detector",
            "local:fake",
            "--detector-base-url",
            "http://example.com/v1",
            "--db",
            db,
        ],
        None,
    );
    assert_eq!(invalid_local.status.code(), Some(2));
}

#[test]
fn model_detector_fails_closed_then_falls_back_to_regex() {
    let home = tempfile::tempdir().unwrap();
    let db = home.path().join("vault.db");
    let db = db.to_str().unwrap();
    let Some((detector_url, detector_rx, detector_worker)) = fake_server(3, |_| chat("invalid detector JSON")) else {
        return;
    };
    let Some((frontier_url, frontier_rx, frontier_worker)) = fake_server(1, |_| chat("Done")) else {
        return;
    };
    let detector_base = format!("{detector_url}/v1");
    let frontier_base = format!("{frontier_url}/v1");
    let args = [
        "shield",
        "ask",
        "Email zora@fiction.example",
        "--upstream",
        "custom",
        "--upstream-base-url",
        &frontier_base,
        "--upstream-key-env",
        "TEST_FRONTIER_KEY",
        "--model",
        "fake",
        "--detector",
        "local:fake",
        "--detector-base-url",
        &detector_base,
        "--db",
        db,
        "--json",
    ];
    let failed = cli(home.path(), &args, None);
    assert!(!failed.status.success());
    assert!(frontier_rx.try_recv().is_err());
    let mut fallback = args.to_vec();
    fallback.push("--detector-fallback");
    fallback.push("regex");
    let result = cli(home.path(), &fallback, None);
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    assert_eq!(val(&result)["audit"]["detector_fallback"], "regex");
    assert!(frontier_rx.recv_timeout(Duration::from_secs(2)).is_ok());
    for _ in 0..3 {
        assert!(detector_rx.recv_timeout(Duration::from_secs(2)).is_ok());
    }
    detector_worker.join().unwrap();
    frontier_worker.join().unwrap();
}

#[test]
fn model_detector_masks_spans_and_tripwire_blocks_residual() {
    let home = tempfile::tempdir().unwrap();
    let db = home.path().join("vault.db");
    let db = db.to_str().unwrap();
    let Some((detector_url, rx, worker)) = fake_server(2, |request| {
        let text = request["messages"][1]["content"].as_str().unwrap();
        if text.contains("123 Fictional Lane") {
            chat(r#"{"entities":[{"kind":"ADDRESS","text":"123 Fictional Lane"}]}"#)
        } else {
            chat(r#"{"entities":[]}"#)
        }
    }) else {
        return;
    };
    let Some((frontier_url, frontier_rx, frontier_worker)) = fake_server(1, |request| {
        let text = request["messages"][1]["content"].as_str().unwrap();
        assert!(text.contains("ADDRESS_"));
        assert!(!text.contains("123 Fictional Lane"));
        chat("Okay")
    }) else {
        return;
    };
    let detector_base = format!("{detector_url}/v1");
    let frontier_base = format!("{frontier_url}/v1");
    let result = cli(
        home.path(),
        &[
            "shield",
            "ask",
            "Summarize 123 Fictional Lane",
            "--upstream",
            "custom",
            "--upstream-base-url",
            &frontier_base,
            "--upstream-key-env",
            "TEST_FRONTIER_KEY",
            "--model",
            "fake",
            "--detector",
            "local:fake",
            "--detector-base-url",
            &detector_base,
            "--db",
            db,
            "--json",
        ],
        None,
    );
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    assert!(frontier_rx.recv_timeout(Duration::from_secs(2)).is_ok());
    for _ in 0..2 {
        assert!(rx.recv_timeout(Duration::from_secs(2)).is_ok());
    }
    worker.join().unwrap();
    frontier_worker.join().unwrap();

    let Some((detector_url, rx, worker)) = fake_server(2, {
        let mut calls = 0;
        move |_| {
            calls += 1;
            if calls == 1 {
                chat(r#"{"entities":[]}"#)
            } else {
                chat(r#"{"entities":[{"kind":"PERSON","text":"milo paperboat"}]}"#)
            }
        }
    }) else {
        return;
    };
    let detector_base = format!("{detector_url}/v1");
    let blocked = cli(
        home.path(),
        &[
            "shield",
            "ask",
            "Summarize milo paperboat",
            "--upstream",
            "custom",
            "--upstream-base-url",
            "http://127.0.0.1:9/v1",
            "--upstream-key-env",
            "TEST_FRONTIER_KEY",
            "--model",
            "fake",
            "--detector",
            "local:fake",
            "--detector-base-url",
            &detector_base,
            "--db",
            db,
            "--json",
        ],
        None,
    );
    assert_eq!(
        blocked.status.code(),
        Some(6),
        "{}",
        String::from_utf8_lossy(&blocked.stderr)
    );
    for _ in 0..2 {
        assert!(rx.recv_timeout(Duration::from_secs(2)).is_ok());
    }
    worker.join().unwrap();
}

#[test]
fn eval_compares_regex_with_model_and_continues_after_record_error() {
    let home = tempfile::tempdir().unwrap();
    let sample = home.path().join("sample.jsonl");
    fs::write(&sample, concat!(
        "{\"id\":\"one\",\"text\":\"milo paperboat\",\"entities\":[{\"kind\":\"PERSON\",\"text\":\"milo paperboat\"}]}\n",
        "{\"id\":\"two\",\"text\":\"Zora Quill\",\"entities\":[{\"kind\":\"PERSON\",\"text\":\"Zora Quill\"}]}\n"
    )).unwrap();
    let Some((detector_url, rx, worker)) = fake_server(2, |request| {
        let text = request["messages"][1]["content"].as_str().unwrap();
        if text.contains("milo paperboat") {
            chat(r#"{"entities":[{"kind":"PERSON","text":"milo paperboat"}]}"#)
        } else {
            chat(r#"{"entities":[]}"#)
        }
    }) else {
        return;
    };
    let base = format!("{detector_url}/v1");
    let sample = sample.to_str().unwrap();
    let output = cli(
        home.path(),
        &[
            "shield",
            "eval",
            "--sample",
            sample,
            "--detector",
            "regex",
            "--detector",
            "local:fake",
            "--detector-base-url",
            &base,
            "--min-recall",
            "0",
            "--json",
        ],
        None,
    );
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let result = val(&output);
    assert_eq!(result["results"].as_array().unwrap().len(), 2);
    assert!(
        result["results"][1]["overall"]["recall"].as_f64().unwrap()
            > result["results"][0]["overall"]["recall"].as_f64().unwrap()
    );
    for _ in 0..2 {
        assert!(rx.recv_timeout(Duration::from_secs(2)).is_ok());
    }
    worker.join().unwrap();

    let Some((detector_url, rx, worker)) = fake_server(2, {
        let mut n = 0;
        move |_| {
            n += 1;
            if n == 1 {
                chat("bad response")
            } else {
                chat(r#"{"entities":[]}"#)
            }
        }
    }) else {
        return;
    };
    let base = format!("{detector_url}/v1");
    let output = cli(
        home.path(),
        &[
            "shield",
            "eval",
            "--sample",
            sample,
            "--detector",
            "local:fake",
            "--detector-base-url",
            &base,
            "--min-recall",
            "0",
            "--json",
        ],
        None,
    );
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let result = val(&output)["results"][0].clone();
    assert_eq!(result["errors"], 1);
    assert_eq!(result["overall"]["counts"]["gold"], 2);
    assert_eq!(result["overall"]["counts"]["found"], 1);
    for _ in 0..2 {
        assert!(rx.recv_timeout(Duration::from_secs(2)).is_ok());
    }
    worker.join().unwrap();
}

#[test]
fn human_ask_prints_answer_on_stdout_and_receipt_on_stderr() {
    let home = tempfile::tempdir().unwrap();
    let db = home.path().join("vault.db");
    let db = db.to_str().unwrap();
    let Some((url, _, worker)) = fake_server(1, |request| {
        let text = request["messages"][1]["content"].as_str().unwrap();
        let token = regex::Regex::new(r"EMAIL_[0-9a-f]{8}")
            .unwrap()
            .find(text)
            .unwrap()
            .as_str();
        chat(&format!("Reply to {token}"))
    }) else {
        return;
    };
    let base = format!("{url}/v1");
    let output = cli(
        home.path(),
        &[
            "shield",
            "ask",
            "Email zora@fiction.example",
            "--upstream",
            "custom",
            "--accept-regex-only",
            "--upstream-base-url",
            &base,
            "--upstream-key-env",
            "TEST_FRONTIER_KEY",
            "--model",
            "fake",
            "--db",
            db,
            "--show-sent",
        ],
        None,
    );
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert_eq!(stdout, "Reply to zora@fiction.example\n");
    assert!(stderr.contains("masked 1 before sending (1 EMAIL), restored 1 locally · receipt audit_"));
    assert!(stderr.contains("sent to 127.0.0.1:"));
    assert!(!stderr.contains("zora@fiction.example"));
    worker.join().unwrap();
}

fn fake_status_server(
    responses: Vec<(u16, &'static str, Value)>,
) -> Option<(String, mpsc::Receiver<Value>, thread::JoinHandle<()>)> {
    let listener = match TcpListener::bind("127.0.0.1:0") {
        Ok(listener) => listener,
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            eprintln!("fake HTTP test unverified: loopback binding denied by sandbox");
            return None;
        }
        Err(e) => panic!("loopback binding failed: {e}"),
    };
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (tx, rx) = mpsc::channel();
    let worker = thread::spawn(move || {
        for (status, after, response) in responses {
            let (mut stream, _) = listener.accept().unwrap();
            let request = body(&stream);
            let _ = tx.send(request); // the test may not read requests
            let text = response.to_string();
            write!(stream, "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nRetry-After: {after}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}", text.len()).unwrap();
        }
    });
    Some((url, rx, worker))
}

#[test]
fn external_upstream_retries_429_and_classifies_402() {
    let home = tempfile::tempdir().unwrap();
    let db = home.path().join("vault.db");
    let db = db.to_str().unwrap();
    let Some((url, rx, worker)) =
        fake_status_server(vec![(429, "1", json!({"error":"slow down"})), (200, "", chat("done"))])
    else {
        return;
    };
    let base = format!("{url}/v1");
    let start = std::time::Instant::now();
    let output = cli(
        home.path(),
        &[
            "shield",
            "ask",
            "hi",
            "--upstream",
            "custom",
            "--accept-regex-only",
            "--upstream-base-url",
            &base,
            "--upstream-key-env",
            "TEST_FRONTIER_KEY",
            "--model",
            "fake",
            "--db",
            db,
            "--json",
        ],
        None,
    );
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(start.elapsed() >= Duration::from_secs(1));
    assert_eq!(val(&output)["answer"], "done");
    for _ in 0..2 {
        assert!(rx.recv_timeout(Duration::from_secs(2)).is_ok());
    }
    worker.join().unwrap();

    let Some((url, rx, worker)) = fake_status_server(vec![(402, "2", json!({"error":"insufficient prepaid credits"}))])
    else {
        return;
    };
    let base = format!("{url}/v1");
    let output = cli(
        home.path(),
        &[
            "shield",
            "ask",
            "hi",
            "--upstream",
            "custom",
            "--accept-regex-only",
            "--upstream-base-url",
            &base,
            "--upstream-key-env",
            "TEST_FRONTIER_KEY",
            "--model",
            "fake",
            "--db",
            db,
            "--json",
        ],
        None,
    );
    assert_eq!(output.status.code(), Some(8));
    assert!(String::from_utf8_lossy(&output.stderr).contains("insufficient prepaid credits"));
    assert!(rx.recv_timeout(Duration::from_secs(2)).is_ok());
    worker.join().unwrap();
}

#[test]
fn ask_does_not_restore_tokens_left_by_earlier_requests() {
    let home = tempfile::tempdir().unwrap();
    let db = home.path().join("vault.db");
    let db = db.to_str().unwrap();
    let data = home.path().join("old.txt");
    fs::write(&data, "old@fiction.example").unwrap();
    let old = cli(
        home.path(),
        &["shield", "redact", data.to_str().unwrap(), "--db", db, "--json"],
        None,
    );
    assert!(old.status.success());
    let old_token = regex::Regex::new(r"EMAIL_[0-9a-f]{8}")
        .unwrap()
        .find(val(&old)["text"].as_str().unwrap())
        .unwrap()
        .as_str()
        .to_string();
    let token_for_reply = old_token.clone();
    let Some((url, _, worker)) = fake_server(1, move |request| {
        let text = request["messages"][1]["content"].as_str().unwrap();
        let current = regex::Regex::new(r"EMAIL_[0-9a-f]{8}")
            .unwrap()
            .find(text)
            .unwrap()
            .as_str();
        chat(&format!("{current} and {token_for_reply}"))
    }) else {
        return;
    };
    let base = format!("{url}/v1");
    let result = cli(
        home.path(),
        &[
            "shield",
            "ask",
            "Email new@fiction.example",
            "--upstream",
            "custom",
            "--accept-regex-only",
            "--upstream-base-url",
            &base,
            "--upstream-key-env",
            "TEST_FRONTIER_KEY",
            "--model",
            "fake",
            "--db",
            db,
            "--json",
        ],
        None,
    );
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    let result = val(&result);
    let answer = result["answer"].as_str().unwrap();
    assert!(answer.contains("new@fiction.example"));
    assert!(answer.contains(&old_token));
    assert!(!answer.contains("old@fiction.example"));
    assert_eq!(result["restored_count"], 1);
    assert_eq!(result["unrestored_count"], 1);
    worker.join().unwrap();
}
