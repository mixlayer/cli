use super::*;
use rusqlite::params;
use std::sync::{Arc, Barrier};

fn fresh() -> Result<(tempfile::TempDir, Store)> {
    let dir = tempfile::tempdir()?;
    let store = Store::open(dir.path().join("data.db"))?;
    Ok((dir, store))
}
fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("parity/fixtures/go-ledger.db")
}

#[test]
fn go_fixture_roundtrip() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("data.db");
    fs::copy(fixture_path(), &path)?;
    let mut store = Store::open(&path)?;
    assert_eq!(store.schema_version()?, 4);
    let runs = store.recent_runs(20)?;
    assert_eq!(runs.len(), 4);
    let expected = [
        (
            "run_195135cc47896eac",
            "Say pong",
            "Pong",
            "qwen/qwen3.5-4b-free",
            13,
            3,
            16,
            0.0,
        ),
        (
            "run_3c8e064d32994477",
            "2+2? number only",
            "4",
            "qwen/qwen3.5-4b-free",
            17,
            2,
            19,
            0.0,
        ),
        (
            "run_e7b91a626a4f904f",
            "2+2? number only",
            "4",
            "qwen/qwen3.5-9b",
            17,
            2,
            19,
            0.0000025,
        ),
        (
            "run_edfaff876a1510e7",
            "Question: Draft one line to PERSON_2f4933f9 (EMAIL_61cd34c8)",
            "I cannot draft a message to PERSON_2f4933f9 (EMAIL_61cd34c8) without providing the content of the message you wish to send. Please specify what you want to say, and I will generate the line for you while preserving the placeholders exactly as required.",
            "qwen/qwen3.5-4b-free",
            78,
            64,
            142,
            0.0,
        ),
    ];
    for (id, prompt, answer, model, prompt_tokens, completion_tokens, total_tokens, cost) in expected {
        let run = runs.iter().find(|r| r.id == id).unwrap();
        assert_eq!(
            (run.prompt.as_str(), run.answer.as_str(), run.model.as_str()),
            (prompt, answer, model)
        );
        assert_eq!(
            (run.prompt_tokens, run.completion_tokens, run.total_tokens),
            (prompt_tokens, completion_tokens, total_tokens)
        );
        assert!((run.cost_usd - cost).abs() < 1e-12);
        assert!(!run.cost_unknown);
    }
    let vault = store.vault_token_map()?;
    assert_eq!(vault.len(), 2);
    assert_eq!(vault.get("PERSON_2f4933f9").map(String::as_str), Some("Jordan Avery"));
    assert_eq!(
        vault.get("EMAIL_61cd34c8").map(String::as_str),
        Some("jordan@acme.example")
    );
    let audit = store.audit_records("", 20)?;
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0].command, "shield ask");
    assert_eq!(audit[0].model, "qwen/qwen3.5-4b-free");
    assert_eq!(audit[0].masked_entities, 2);
    assert_eq!(audit[0].cost_usd, Some(0.0));
    assert_eq!(audit[0].masked_counts.as_ref().unwrap().get("PERSON"), Some(&1));
    assert_eq!(
        store.dictionary_terms()?,
        vec![DictionaryTerm {
            term: "Northwind Traders".into(),
            kind: "ORG".into()
        }]
    );
    assert_eq!(store.ladders()?.len(), 1);
    assert!(store.cached_live_catalog("models", Duration::from_secs(0))?.is_none());
    store.save_run(RunRecord {
        id: "rust_run".into(),
        command: "ask".into(),
        prompt: "Rust Go roundtrip".into(),
        answer: "works".into(),
        model: "test/model".into(),
        prompt_tokens: 7,
        completion_tokens: 1,
        total_tokens: 8,
        cost_usd: 0.25,
        ..Default::default()
    })?;
    let new_vault = store.ensure_vault_entry("EMAIL", "rust@example.com")?;
    drop(store);
    let store = Store::open(&path)?;
    assert_eq!(store.recent_runs(20)?.len(), 5);
    assert_eq!(store.get_run("rust_run")?.unwrap().answer, "works");
    assert_eq!(
        store.vault_token_map()?.get(&new_vault.token),
        Some(&"rust@example.com".to_owned())
    );
    assert_eq!(store.audit_records("", 20)?.len(), 1);
    Ok(())
}

fn normalize_sql(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}
fn schema_rows(conn: &Connection) -> Result<Vec<(String, String, String)>> {
    let mut stmt = conn.prepare("SELECT type,name,sql FROM sqlite_master WHERE sql IS NOT NULL ORDER BY type,name")?;
    Ok(stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .map(|(t, n, s): (String, String, String)| (t, n, normalize_sql(&s)))
        .collect())
}
fn schema_dump_rows() -> Vec<(String, String, String)> {
    let dump = include_str!("../../parity/fixtures/go-schema.sql");
    dump.split(';')
        .filter_map(|part| {
            let part = part.trim();
            if part.is_empty() {
                return None;
            }
            let sql = part.split("/*").next().unwrap_or(part).trim();
            let words: Vec<&str> = sql.split_whitespace().collect();
            if words.len() < 3 {
                return None;
            }
            let (typ, name) = if words[1] == "VIRTUAL" {
                ("table", words[3])
            } else if words[1] == "TABLE" {
                ("table", words[2])
            } else {
                ("index", words[2])
            };
            Some((
                typ.into(),
                name.split('(').next().unwrap_or(name).trim_matches('\'').into(),
                normalize_sql(sql),
            ))
        })
        .collect()
}
#[test]
fn schema_matches_go() -> Result<()> {
    let (_dir, store) = fresh()?;
    assert_eq!(store.schema_version()?, 4);
    let actual = schema_rows(store.connection())?;
    let mut expected = schema_dump_rows();
    expected.sort();
    assert_eq!(actual, expected);
    Ok(())
}

#[test]
fn run_ledger_search_and_vault() -> Result<()> {
    let (_dir, mut s) = fresh()?;
    s.save_run(RunRecord {
        id: "run_test".into(),
        command: "ask".into(),
        prompt: "Find revenue leakage".into(),
        answer: "Check churn cohorts".into(),
        reasoning: "Revenue changed after discounting".into(),
        model: "qwen/qwen3.5-4b-free".into(),
        raw_json: Some(serde_json::json!({"ok":true})),
        ..Default::default()
    })?;
    assert_eq!(s.search_runs("revenue", 10)?[0].id, "run_test");
    assert_eq!(s.recent_runs(0)?.len(), 1);
    assert_eq!(
        s.get_run("run_test")?.unwrap().raw_json,
        Some(serde_json::json!({"ok":true}))
    );
    s.save_entry(VaultEntry {
        token: "EMAIL_1".into(),
        value: "jordan@example.com".into(),
        kind: "EMAIL".into(),
    })?;
    assert_eq!(s.token_for_value("jordan@example.com")?.unwrap().token, "EMAIL_1");
    assert_eq!(s.token_map()?.get("EMAIL_1"), Some(&"jordan@example.com".into()));
    Ok(())
}
#[test]
fn id_and_vault_tokens() -> Result<()> {
    let mut seen = std::collections::HashSet::new();
    for _ in 0..10_000 {
        assert!(seen.insert(new_id("run")?));
    }
    let (_dir, mut s) = fresh()?;
    let e = s.ensure_vault_entry("EMAIL", "agent@example.com")?;
    assert!(Regex::new(r"^EMAIL_[0-9a-f]{8}$")?.is_match(&e.token));
    assert_eq!(s.ensure_vault_entry("PERSON", " agent@example.com ")?.token, e.token);
    assert_eq!(s.vault_entries(false)?[0].value, "");
    assert!(!s.vault_rotation_status().0);
    assert!(s.vault_token_map()?.contains_key(&e.token));
    assert_eq!(s.purge_vault()?, 1);
    Ok(())
}
#[test]
fn concurrent_vault_handles() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("data.db");
    Store::open(&path)?;
    let barrier = Arc::new(Barrier::new(8));
    let threads: Vec<_> = (0..8)
        .map(|i| {
            let path = path.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || -> Result<String> {
                let mut s = Store::open(path)?;
                barrier.wait();
                Ok(s.ensure_vault_entry("EMAIL", &format!("person{i}@example.com"))?.token)
            })
        })
        .collect();
    let mut tokens = std::collections::HashSet::new();
    for thread in threads {
        assert!(tokens.insert(thread.join().unwrap()?));
    }
    assert_eq!(tokens.len(), 8);
    Ok(())
}
#[test]
fn version_pragmas_read_only_and_permissions() -> Result<()> {
    let (dir, s) = fresh()?;
    let path = dir.path().join("data.db");
    assert_eq!(s.schema_version()?, 4);
    let journal: String = s.connection().pragma_query_value(None, "journal_mode", |r| r.get(0))?;
    let busy: i64 = s.connection().pragma_query_value(None, "busy_timeout", |r| r.get(0))?;
    assert_eq!(journal.to_lowercase(), "wal");
    assert_eq!(busy, 5000);
    let ro = Store::open_read_only(&path)?;
    assert_eq!(ro.schema_version()?, 4);
    let ro_busy: i64 = ro.connection().pragma_query_value(None, "busy_timeout", |r| r.get(0))?;
    assert_eq!(ro_busy, 5000);
    for write in [
        "INSERT INTO runs (id,command,prompt) VALUES ('x','ask','x')",
        "UPDATE runs SET command='changed'",
        "DELETE FROM runs",
        "REPLACE INTO runs (id,command,prompt) VALUES ('x','ask','x')",
        "WITH r AS (SELECT 'x' AS id) INSERT INTO runs (id,command,prompt) SELECT id,'ask','x' FROM r",
    ] {
        assert!(ro.connection().execute(write, []).is_err(), "{write}");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(fs::metadata(&path)?.permissions().mode() & 0o777, 0o600);
        // The store creates missing parent directories as 0700 (Go: MkdirAll 0o700).
        let nested = dir.path().join("created-by-store").join("data.db");
        drop(Store::open(&nested)?);
        assert_eq!(
            fs::metadata(nested.parent().unwrap())?.permissions().mode() & 0o777,
            0o700
        );
        for sidecar in [format!("{}-wal", path.display()), format!("{}-shm", path.display())] {
            if let Ok(meta) = fs::metadata(sidecar) {
                assert_eq!(meta.permissions().mode() & 0o777, 0o600);
            }
        }
    }
    drop(ro);
    drop(s);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644))?;
        let reopened = Store::open(&path)?;
        assert_eq!(fs::metadata(reopened.path())?.permissions().mode() & 0o777, 0o600);
    }
    Ok(())
}
#[test]
fn read_only_delete_mode_and_newer_version() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("data.db");
    let conn = Connection::open(&path)?;
    conn.execute("CREATE TABLE resources (id TEXT)", [])?;
    drop(conn);
    let ro = Store::open_read_only(&path)?;
    assert_eq!(
        ro.connection()
            .query_row("SELECT count(*) FROM resources", [], |r| r.get::<_, i64>(0))?,
        0
    );
    drop(ro);
    let conn = Connection::open(&path)?;
    conn.pragma_update(None, "user_version", 999)?;
    drop(conn);
    assert!(
        Store::open(&path)
            .err()
            .unwrap()
            .to_string()
            .contains("newer than supported")
    );
    Ok(())
}
#[test]
fn migration_preserves_audit_and_adds_columns() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("legacy.db");
    let conn = Connection::open(&path)?;
    conn.execute_batch("CREATE TABLE audit (id TEXT PRIMARY KEY,run_id TEXT,command TEXT NOT NULL,payload_sha256 TEXT NOT NULL,byte_count INTEGER NOT NULL,model TEXT,guard_model TEXT,masked_entities INTEGER DEFAULT 0,leaked_pii_count INTEGER DEFAULT 0,cost_usd REAL DEFAULT 0,created_at DATETIME DEFAULT CURRENT_TIMESTAMP); INSERT INTO audit(id,command,payload_sha256,byte_count,model,masked_entities,leaked_pii_count,cost_usd,created_at) VALUES('old','shield ask','abc',10,'legacy',1,0,0.01,'2026-01-01T00:00:00Z'); CREATE TABLE sync_state (resource_type TEXT PRIMARY KEY); PRAGMA user_version=4;")?;
    drop(conn);
    let mut s = Store::open(&path)?;
    let old = s.audit_records("old", 1)?;
    assert_eq!(old[0].model, "legacy");
    assert_eq!(old[0].cost_usd, Some(0.01));
    s.save_audit(AuditRecord {
        id: "new".into(),
        command: "shield ask".into(),
        payload_sha256: "def".into(),
        byte_count: 5,
        upstream_host: "api.example.test".into(),
        masked_counts: Some(HashMap::from([("PERSON".into(), 1)])),
        ..Default::default()
    })?;
    let new = s.audit_records("new", 1)?;
    assert_eq!(new[0].upstream_host, "api.example.test");
    assert_eq!(new[0].masked_counts.as_ref().unwrap().get("PERSON"), Some(&1));
    assert_eq!(new[0].cost_usd, None);
    let cols: Vec<String> = s
        .connection()
        .prepare("PRAGMA table_info(sync_state)")?
        .query_map([], |r| r.get(1))?
        .collect::<rusqlite::Result<_>>()?;
    assert!(cols.contains(&"last_cursor".into()));
    assert!(cols.contains(&"total_count".into()));
    Ok(())
}
#[test]
fn dictionary_models_catalog_and_ladder() -> Result<()> {
    let (_dir, mut s) = fresh()?;
    s.add_dictionary_term(" Northwind Traders ", "ORG")?;
    s.add_dictionary_term("northwind traders", "PERSON")?;
    assert_eq!(s.dictionary_terms()?.len(), 1);
    assert_eq!(s.dictionary_terms()?[0].kind, "PERSON");
    assert!(s.remove_dictionary_term("NORTHWIND TRADERS")?);
    s.save_model_cache(&ModelCacheRecord {
        id: "vendor/model".into(),
        context_window: 200000,
        supports_tools: true,
        supports_reasoning: false,
        input_price_per_million: 1.0,
        output_price_per_million: 2.0,
        raw_json: Some(serde_json::json!({"test":true})),
        ..Default::default()
    })?;
    assert_eq!(s.query_models("tools ctx>=128k model")?.len(), 1);
    assert!(s.query_models("reasoning")?.is_empty());
    s.save_live_catalog("models", &serde_json::json!([{"id":"vendor/model"}]))?;
    assert_eq!(
        s.cached_live_catalog("models", Duration::from_secs(60))?,
        Some(serde_json::json!([{"id":"vendor/model"}]))
    );
    s.save_ladder(
        "ladder_1",
        "prompt",
        &["model-a".into(), "model-b".into()],
        "model-b",
        "",
    )?;
    assert_eq!(s.ladders()?[0].rungs, vec!["model-a", "model-b"]);
    Ok(())
}
#[test]
fn sql_rejects_writes_and_private_tables() -> Result<()> {
    let (_dir, s) = fresh()?;
    assert_eq!(s.query_read_only("select count(*) as n from runs", true)?[0]["n"], 0);
    for sql in [
        "delete from runs",
        "with x as (select 1) delete from runs",
        "select * from vault",
        "select * from shield_dictionary",
        "select * from sqlite_master",
        "select * from pragma_table_info('runs')",
    ] {
        assert!(s.query_read_only(sql, true).is_err(), "{sql}");
    }
    assert!(s.query_read_only("select * from vault", false).is_ok());
    assert!(s.query_read_only("select 'delete' as word", true).is_ok());
    Ok(())
}
#[test]
fn private_tables_unreadable_through_views() -> Result<()> {
    let (_dir, mut s) = fresh()?;
    s.ensure_vault_entry("EMAIL", "jordan@acme.example")?;
    s.conn.execute_batch(
        "CREATE VIEW saved AS SELECT value FROM vault;
         CREATE VIEW terms AS SELECT * FROM shield_dictionary;
         CREATE VIEW nested AS SELECT * FROM saved;
         CREATE VIEW schema_rows AS SELECT sql FROM sqlite_master;
         CREATE VIEW recent AS SELECT id FROM runs;",
    )?;
    for sql in [
        "select * from saved",
        "select * from terms",
        "select * from nested",
        "select * from schema_rows",
        "select (select value from saved limit 1) as v",
        "with x as (select * from nested) select * from x",
    ] {
        let err = s.query_read_only(sql, true).expect_err(sql).to_string();
        assert!(err.contains("not readable in agent mode"), "{sql}: {err}");
    }
    // Ordinary views and the human CLI still work.
    assert!(s.query_read_only("select * from recent", true).is_ok());
    assert_eq!(s.query_read_only("select * from saved", false)?.len(), 1);
    // The guard is removed after each query.
    assert!(s.conn.prepare("select value from vault").is_ok());
    Ok(())
}
#[test]
fn parse_go_timestamps() {
    assert!(parse_stored_time("2026-01-02T03:04:05.123456789Z").is_some());
    assert!(parse_stored_time("2026-01-02 03:04:05.123456789 -0600 CST").is_some());
    assert!(parse_stored_time("2026-01-02 03:04:05 -0600 CST").is_some());
    assert_eq!(parse_stored_time("invalid"), None);
}
#[test]
fn resources_v3_fts_rebuild() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("data.db");
    let conn = Connection::open(&path)?;
    conn.execute_batch("CREATE TABLE resources (id TEXT NOT NULL,resource_type TEXT NOT NULL,data JSON NOT NULL,synced_at DATETIME DEFAULT CURRENT_TIMESTAMP,updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,PRIMARY KEY(resource_type,id)); CREATE VIRTUAL TABLE resources_fts USING fts5(id,resource_type,content,tokenize='porter unicode61'); INSERT INTO resources(id,resource_type,data) VALUES('shared','models','{\"name\":\"Alpha Searchable\",\"id\":\"hidden\"}'),('shared','providers','{\"name\":\"Beta Searchable\"}'); PRAGMA user_version=3;")?;
    drop(conn);
    let s = Store::open(&path)?;
    assert_eq!(s.schema_version()?, 4);
    let count: i64 = s.connection().query_row(
        "SELECT count(*) FROM resources_fts WHERE resources_fts MATCH 'Searchable'",
        [],
        |r| r.get(0),
    )?;
    assert_eq!(count, 2);
    assert_eq!(
        s.connection().query_row(
            "SELECT count(*) FROM resources_fts WHERE resources_fts MATCH 'hidden'",
            [],
            |r| r.get::<_, i64>(0)
        )?,
        0
    );
    Ok(())
}

#[test]
fn fresh_zero_version_and_reopen_idempotent() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("data.db");
    let conn = Connection::open(&path)?;
    conn.pragma_update(None, "user_version", 0)?;
    drop(conn);
    let s = Store::open(&path)?;
    assert_eq!(s.schema_version()?, 4);
    let before = schema_rows(s.connection())?;
    drop(s);
    let s = Store::open(&path)?;
    assert_eq!(s.schema_version()?, 4);
    assert_eq!(before, schema_rows(s.connection())?);
    Ok(())
}

#[test]
fn concurrent_fresh_migrations() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("data.db");
    let barrier = Arc::new(Barrier::new(8));
    let threads: Vec<_> = (0..8)
        .map(|_| {
            let path = path.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || -> Result<i64> {
                barrier.wait();
                Store::open(path)?.schema_version()
            })
        })
        .collect();
    for thread in threads {
        assert_eq!(thread.join().unwrap()?, 4);
    }
    Ok(())
}

#[test]
fn v4_resources_fts_does_not_rebuild() -> Result<()> {
    let (_dir, s) = fresh()?;
    let path = s.path().to_owned();
    s.connection().execute(
        "INSERT INTO resources(id,resource_type,data) VALUES (?,?,?)",
        params!["shared", "biz", r#"{"name":"canonical resource"}"#],
    )?;
    s.connection().execute(
        "INSERT INTO resources_fts (rowid,id,resource_type,content) VALUES (?,?,?,?)",
        params![schema::fts_rowid("biz", "shared"), "shared", "biz", "sentinel fts"],
    )?;
    drop(s);
    let s = Store::open(path)?;
    let content: String = s
        .connection()
        .query_row("SELECT content FROM resources_fts WHERE id='shared'", [], |r| r.get(0))?;
    assert_eq!(content, "sentinel fts");
    Ok(())
}

#[test]
fn v1_resource_composite_key_upgrade() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("data.db");
    let conn = Connection::open(&path)?;
    conn.execute_batch("CREATE TABLE resources (id TEXT PRIMARY KEY, resource_type TEXT NOT NULL, data JSON NOT NULL, synced_at DATETIME DEFAULT CURRENT_TIMESTAMP, updated_at DATETIME DEFAULT CURRENT_TIMESTAMP); INSERT INTO resources (id,resource_type,data) VALUES ('shared','biz','{\"name\":\"Alpha Searchable\"}'); PRAGMA user_version=1;")?;
    drop(conn);
    let s = Store::open(&path)?;
    assert_eq!(s.schema_version()?, 4);
    s.connection().execute(
        "INSERT INTO resources (id,resource_type,data) VALUES ('shared','other','{}')",
        [],
    )?;
    assert_eq!(
        s.connection()
            .query_row("SELECT COUNT(*) FROM resources WHERE id='shared'", [], |r| r
                .get::<_, i64>(0))?,
        2
    );
    assert_eq!(
        s.connection().query_row(
            "SELECT COUNT(*) FROM resources_fts WHERE resources_fts MATCH 'Searchable'",
            [],
            |r| r.get::<_, i64>(0)
        )?,
        1
    );
    Ok(())
}

#[test]
fn newer_schema_rejected_under_writer_lock() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("data.db");
    let seed = Store::open(&path)?;
    drop(seed);
    let conn = Connection::open(&path)?;
    conn.pragma_update(None, "user_version", 999)?;
    conn.execute_batch("BEGIN IMMEDIATE; CREATE TABLE IF NOT EXISTS holder_lock (id INTEGER)")?;
    let start = std::time::Instant::now();
    let error = Store::open(&path).err().unwrap();
    assert!(error.to_string().contains("newer than supported"));
    assert!(start.elapsed() < Duration::from_secs(5));
    conn.execute_batch("ROLLBACK")?;
    Ok(())
}

/// Writes a Rust-created database for the Go CLI to read. Run with
/// `MIXLAYER_XCHECK_HOME=<dir> cargo test write_db_for_go -- --ignored`.
#[test]
#[ignore]
fn write_db_for_go() -> Result<()> {
    use crate::shield::Vault;
    let home = PathBuf::from(std::env::var("MIXLAYER_XCHECK_HOME").expect("MIXLAYER_XCHECK_HOME"));
    let mut s = Store::open(home.join("data").join("data.db"))?;
    s.save_run(RunRecord {
        command: "ask".into(),
        prompt: "rust wrote this prompt about pelicans".into(),
        answer: "rust answer".into(),
        model: "qwen/qwen3.5-9b".into(),
        total_tokens: 12,
        cost_usd: 0.000003,
        ..Default::default()
    })?;
    s.ensure_entry("EMAIL", "rust@example.test")?;
    s.add_dictionary_term("Rust Labs", "ORG")?;
    Ok(())
}
