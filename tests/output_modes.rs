use std::path::Path;
use std::process::{Command, Output};

use mixlayer::store::{ModelCacheRecord, Store};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const BASE: &str = "http://127.0.0.1:9";
const AUTH: &str = "Bearer output-test";

fn cli(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_mixlayer"))
        .args(args)
        .env_clear()
        .env("HOME", home)
        .env("MIXLAYER_HOME", home)
        .env("MIXLAYER_BASE_URL", format!("{BASE}/v1"))
        .env("MIXLAYER_PLATFORM_URL", BASE)
        .env("MIXLAYER_API_KEY", "output-test")
        .env("MIXLAYER_NO_AGENT_DETECT", "1")
        .env("NO_COLOR", "1")
        .output()
        .unwrap()
}

fn fixture(case: usize) -> String {
    std::fs::read_to_string(format!(
        "{}/parity/fixtures/go-output/expected/{case}.out",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn assert_case(output: Output, case: usize) {
    assert_eq!(
        format!(
            "{}exit={}\n",
            String::from_utf8(output.stdout).unwrap(),
            output.status.code().unwrap()
        ),
        fixture(case),
        "case {case}"
    );
    assert!(
        output.stderr.is_empty(),
        "case {case}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn endpoint_commands_preserve_cached_wire_output() {
    let home = tempfile::tempdir().unwrap();
    // Seed the existing HTTP cache so command wiring is testable without sockets.
    let key = format!(
        "/models|base_url={BASE}/v1|auth_source=env:MIXLAYER_API_KEY|auth={}|config_path={}",
        hex::encode(&Sha256::digest(AUTH.as_bytes())[..8]),
        home.path().join("config/config.toml").display()
    );
    let cache = home.path().join("cache/http");
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::write(
        cache.join(format!("{}.json", hex::encode(&Sha256::digest(key.as_bytes())[..8]))),
        r#"{"object": "list", "data": [{"id": "fake/model", "object": "model"}]}"#,
    )
    .unwrap();
    let cases: &[&[&str]] = &[
        &["models", "list"],
        &["models", "list", "--json"],
        &["models", "list", "--csv"],
        &["models", "list", "--plain"],
        &["models", "list", "--quiet"],
        &["models", "list", "--select", "id"],
        &["models", "list", "--compact"],
        &["models", "get", "fake/model"],
        &["models", "get", "fake/model", "--csv"],
        &["models", "get", "fake/model", "--quiet"],
    ];
    for (i, args) in cases.iter().enumerate() {
        assert_case(cli(home.path(), args), i + 1);
    }
    let agent = cli(home.path(), &["models", "list", "--agent", "--quiet"]);
    assert!(agent.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&agent.stdout).unwrap()["meta"]["source"],
        "live"
    );
    let explicit = cli(home.path(), &["models", "list", "--agent", "--json=false", "--quiet"]);
    assert!(explicit.status.success());
    assert!(explicit.stdout.is_empty());
}

#[test]
fn profile_table_flags_match_recorded_quirks() {
    let home = tempfile::tempdir().unwrap();
    assert!(
        cli(home.path(), &["profile", "save", "p", "--json", "--description", "d"])
            .status
            .success()
    );
    for (flag, case) in [("--csv", 20), ("--plain", 21), ("--quiet", 22)] {
        assert_case(cli(home.path(), &["profile", "list", flag]), case);
    }
}

#[test]
fn query_commands_use_default_and_explicit_stores_without_reseeding() {
    let home = tempfile::tempdir().unwrap();
    let custom = home.path().join("custom.db");
    assert_case(cli(home.path(), &["models", "query", "fake"]), 11);
    assert_case(
        cli(
            home.path(),
            &["models", "query", "fake", "--db", custom.to_str().unwrap()],
        ),
        13,
    );
    let mut store = Store::open(&custom).unwrap();
    store
        .save_model_cache(&ModelCacheRecord {
            id: "fake/model".into(),
            raw_json: Some(json!({"id":"fake/model","object":"model"})),
            ..Default::default()
        })
        .unwrap();
    drop(store);
    let out = cli(
        home.path(),
        &["models", "query", "fake", "--db", custom.to_str().unwrap()],
    );
    assert!(out.status.success());
    let mut value: Value = serde_json::from_slice(&out.stdout).unwrap();
    value[0]["updated_at"] = "<time>".into();
    assert_eq!(
        format!("{}\nexit=0\n", serde_json::to_string_pretty(&value).unwrap()),
        fixture(14)
    );
    assert_case(cli(home.path(), &["models", "query", "fake"]), 11);
}

#[test]
fn live_catalog_commands_read_go_shaped_cache_and_fall_back_offline() {
    let home = tempfile::tempdir().unwrap();
    let db = home.path().join("custom.db");
    let raw = fixture(16).replace("exit=0\n", "");
    let mut store = Store::open(&db).unwrap();
    let scope = format!("{BASE}:{}", hex::encode(&Sha256::digest(AUTH.as_bytes())[..8]));
    store
        .save_live_catalog(&scope, &serde_json::from_str(&raw).unwrap())
        .unwrap();
    drop(store);
    assert_case(
        cli(
            home.path(),
            &["models", "list", "--live-prices", "--db", db.to_str().unwrap()],
        ),
        15,
    );
    assert_case(
        cli(
            home.path(),
            &[
                "models",
                "list",
                "--live-prices",
                "--json",
                "--db",
                db.to_str().unwrap(),
            ],
        ),
        16,
    );
    let fallback = cli(
        home.path(),
        &["models", "list", "--live-prices", "--json", "--timeout", "1ms"],
    );
    assert!(fallback.status.success());
    let value: Value = serde_json::from_slice(&fallback.stdout).unwrap();
    assert_eq!(value["source"], "offline_fallback");
    assert_eq!(value["models"].as_array().unwrap().len(), 5);
    assert!(value["warning"].is_string());
}
