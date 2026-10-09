use anyhow::{Context, Result, bail};
use regex::Regex;
use rusqlite::{Connection, TransactionBehavior, params};
use serde_json::Value;
use std::hash::Hasher;
use std::sync::OnceLock;

pub const VERSION: i64 = 4;
const RESOURCES_FTS: &str = "CREATE VIRTUAL TABLE IF NOT EXISTS resources_fts USING fts5(\n\tid, resource_type, content, tokenize='porter unicode61'\n)";

const BASE: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS resources (\n\t\t\tid TEXT NOT NULL,\n\t\t\tresource_type TEXT NOT NULL,\n\t\t\tdata JSON NOT NULL,\n\t\t\tsynced_at DATETIME DEFAULT CURRENT_TIMESTAMP,\n\t\t\tupdated_at DATETIME DEFAULT CURRENT_TIMESTAMP,\n\t\t\tPRIMARY KEY (resource_type, id)\n\t\t)",
    "CREATE INDEX IF NOT EXISTS idx_resources_type ON resources(resource_type)",
    "CREATE INDEX IF NOT EXISTS idx_resources_synced ON resources(synced_at)",
    "CREATE TABLE IF NOT EXISTS sync_state (\n\t\t\tresource_type TEXT PRIMARY KEY,\n\t\t\tlast_cursor TEXT,\n\t\t\tlast_synced_at DATETIME,\n\t\t\ttotal_count INTEGER DEFAULT 0\n\t\t)",
    RESOURCES_FTS,
];
const EXTRA: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS runs (\n\t\t\tid TEXT PRIMARY KEY,\n\t\t\tgroup_id TEXT,\n\t\t\tcommand TEXT NOT NULL,\n\t\t\tprompt TEXT NOT NULL,\n\t\t\tanswer TEXT,\n\t\t\treasoning TEXT,\n\t\t\tmodel TEXT,\n\t\t\tseed INTEGER,\n\t\t\tparams_json JSON,\n\t\t\traw_json JSON,\n\t\t\tprompt_tokens INTEGER DEFAULT 0,\n\t\t\tcompletion_tokens INTEGER DEFAULT 0,\n\t\t\ttotal_tokens INTEGER DEFAULT 0,\n\t\t\tcost_usd REAL DEFAULT 0,\n\t\t\tlatency_ms INTEGER DEFAULT 0,\n\t\t\tcreated_at DATETIME DEFAULT CURRENT_TIMESTAMP\n\t\t)",
    "CREATE VIRTUAL TABLE IF NOT EXISTS runs_fts USING fts5(\n\t\t\tid UNINDEXED, prompt, answer, reasoning, tokenize='porter unicode61'\n\t\t)",
    "CREATE INDEX IF NOT EXISTS idx_runs_created ON runs(created_at)",
    "CREATE INDEX IF NOT EXISTS idx_runs_group ON runs(group_id)",
    "CREATE TABLE IF NOT EXISTS vault (\n\t\t\ttoken TEXT PRIMARY KEY,\n\t\t\tvalue TEXT NOT NULL,\n\t\t\tkind TEXT NOT NULL,\n\t\t\tvalue_hash TEXT NOT NULL UNIQUE,\n\t\t\tcreated_at DATETIME DEFAULT CURRENT_TIMESTAMP,\n\t\t\trotated_at DATETIME\n\t\t)",
    "CREATE INDEX IF NOT EXISTS idx_vault_kind ON vault(kind)",
    "CREATE TABLE IF NOT EXISTS audit (\n\t\t\tid TEXT PRIMARY KEY,\n\t\t\trun_id TEXT,\n\t\t\tcommand TEXT NOT NULL,\n\t\t\tpayload_sha256 TEXT NOT NULL,\n\t\t\tbyte_count INTEGER NOT NULL,\n\t\t\tmodel TEXT,\n\t\t\tguard_model TEXT,\n\t\t\tmasked_entities INTEGER DEFAULT 0,\n\t\t\tleaked_pii_count INTEGER DEFAULT 0,\n\t\t\tcost_usd REAL DEFAULT 0,\n\t\t\tcreated_at DATETIME DEFAULT CURRENT_TIMESTAMP\n\t\t)",
    "CREATE INDEX IF NOT EXISTS idx_audit_created ON audit(created_at)",
    "CREATE TABLE IF NOT EXISTS shield_dictionary (term TEXT PRIMARY KEY COLLATE NOCASE, kind TEXT NOT NULL)",
    "CREATE TABLE IF NOT EXISTS ladders (\n\t\t\tid TEXT PRIMARY KEY,\n\t\t\tprompt TEXT NOT NULL,\n\t\t\trungs TEXT NOT NULL,\n\t\t\tfirst_confident_model TEXT,\n\t\t\tjudge_model TEXT,\n\t\t\tcreated_at DATETIME DEFAULT CURRENT_TIMESTAMP\n\t\t)",
    "CREATE TABLE IF NOT EXISTS models_cache (\n\t\t\tid TEXT PRIMARY KEY,\n\t\t\tcontext_window INTEGER DEFAULT 131072,\n\t\t\tsupports_tools BOOLEAN DEFAULT TRUE,\n\t\t\tsupports_reasoning BOOLEAN DEFAULT TRUE,\n\t\t\tinput_price_per_million REAL DEFAULT 0,\n\t\t\toutput_price_per_million REAL DEFAULT 0,\n\t\t\traw_json JSON,\n\t\t\tupdated_at DATETIME DEFAULT CURRENT_TIMESTAMP\n\t\t)",
    "CREATE TABLE IF NOT EXISTS live_catalog_cache (\n\t\t\tscope TEXT PRIMARY KEY,\n\t\t\tmodels_json TEXT NOT NULL,\n\t\t\tfetched_at DATETIME NOT NULL\n\t\t)",
];
const AUDIT_COLUMNS: &[(&str, &str)] = &[
    ("upstream_host", "TEXT"),
    ("detector", "TEXT"),
    ("detector_host", "TEXT"),
    ("detector_fallback", "TEXT"),
    ("detector_notice", "TEXT"),
    ("tripwire_detector", "TEXT"),
    ("ignored_model_spans", "INTEGER DEFAULT 0"),
    ("detector_cost_usd", "REAL"),
    ("masked_counts", "TEXT"),
    ("restored_count", "INTEGER DEFAULT 0"),
    ("unrestored_count", "INTEGER DEFAULT 0"),
    ("unrestored_tokens", "TEXT"),
    ("repairs", "TEXT"),
];

pub fn migrate(conn: &mut Connection) -> Result<()> {
    let current = version(conn)?;
    if current > VERSION {
        bail!(
            "database schema version {current} is newer than supported version {VERSION}; upgrade the CLI binary or open an older database"
        );
    }
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let current = version(&tx)?;
    if current > VERSION {
        bail!(
            "database schema version {current} is newer than supported version {VERSION}; upgrade the CLI binary or open an older database"
        );
    }
    if current < 2 && table_exists(&tx, "resources")? {
        let mut info = tx.prepare("PRAGMA table_info(resources)")?;
        let keys: Vec<(String, i64)> = info
            .query_map([], |row| Ok((row.get(1)?, row.get(5)?)))?
            .collect::<rusqlite::Result<_>>()?;
        drop(info);
        if !keys.iter().any(|(n, p)| n == "resource_type" && *p == 1) || !keys.iter().any(|(n, p)| n == "id" && *p == 2)
        {
            tx.execute_batch("CREATE TABLE resources_v2 (\n\t\t\tid TEXT NOT NULL,\n\t\t\tresource_type TEXT NOT NULL,\n\t\t\tdata JSON NOT NULL,\n\t\t\tsynced_at DATETIME DEFAULT CURRENT_TIMESTAMP,\n\t\t\tupdated_at DATETIME DEFAULT CURRENT_TIMESTAMP,\n\t\t\tPRIMARY KEY (resource_type, id)\n\t\t); INSERT INTO resources_v2 (id,resource_type,data,synced_at,updated_at) SELECT id,resource_type,data,synced_at,updated_at FROM resources; DROP TABLE resources; ALTER TABLE resources_v2 RENAME TO resources;")?;
        }
    }
    ensure_column(&tx, "sync_state", "last_cursor", "TEXT")?;
    ensure_column(&tx, "sync_state", "last_synced_at", "DATETIME")?;
    ensure_column(&tx, "sync_state", "total_count", "INTEGER DEFAULT 0")?;
    for sql in BASE.iter().chain(EXTRA) {
        tx.execute(sql, []).with_context(|| format!("migration: {sql}"))?;
    }
    for &(name, decl) in AUDIT_COLUMNS {
        ensure_column(&tx, "audit", name, decl)?;
    }
    if current < VERSION && table_exists(&tx, "resources")? {
        tx.execute("DROP TABLE IF EXISTS resources_fts", [])?;
        tx.execute(RESOURCES_FTS, [])?;
        rebuild_resources_fts(&tx)?;
    }
    tx.pragma_update(None, "user_version", VERSION)?;
    tx.commit()?;
    Ok(())
}

pub fn version(conn: &Connection) -> Result<i64> {
    Ok(conn.pragma_query_value(None, "user_version", |row| row.get(0))?)
}
fn table_exists(conn: &Connection, name: &str) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?)",
        [name],
        |row| row.get(0),
    )?)
}
fn ensure_column(conn: &Connection, table: &str, column: &str, decl: &str) -> Result<()> {
    if !table_exists(conn, table)? {
        return Ok(());
    }
    let mut stmt = conn.prepare(&format!("PRAGMA table_info(\"{table}\")"))?;
    let names: Vec<String> = stmt.query_map([], |row| row.get(1))?.collect::<rusqlite::Result<_>>()?;
    drop(stmt);
    if !names.iter().any(|n| n == column) {
        conn.execute(&format!("ALTER TABLE \"{table}\" ADD COLUMN \"{column}\" {decl}"), [])?;
    }
    Ok(())
}
fn rebuild_resources_fts(conn: &Connection) -> Result<()> {
    let mut stmt = conn.prepare("SELECT id, resource_type, data FROM resources")?;
    let rows: Vec<(String, String, String)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    drop(stmt);
    for (id, scope, data) in rows {
        let content = searchable_content(&data);
        conn.execute(
            "INSERT INTO resources_fts (rowid,id,resource_type,content) VALUES (?,?,?,?)",
            params![fts_rowid(&scope, &id), id, scope, content],
        )?;
    }
    Ok(())
}
pub(super) fn fts_rowid(scope: &str, id: &str) -> i64 {
    // Go's hash/fnv New64a, with a NUL separator.
    let mut h = Fnv64a(0xcbf29ce484222325);
    h.write(scope.as_bytes());
    h.write(&[0]);
    h.write(id.as_bytes());
    (h.finish() & 0x7fff_ffff_ffff_ffff) as i64
}
struct Fnv64a(u64);
impl Hasher for Fnv64a {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.0 = (self.0 ^ u64::from(*b)).wrapping_mul(0x100000001b3);
        }
    }
}
fn searchable_content(data: &str) -> String {
    fn collect(value: &Value, key: &str, out: &mut Vec<String>) {
        match value {
            Value::Object(m) => {
                for (k, v) in m {
                    collect(v, k, out);
                }
            }
            Value::Array(a) => {
                for v in a {
                    collect(v, key, out);
                }
            }
            Value::String(s) => {
                let s = s.trim();
                let k = key.to_lowercase();
                let lower = s.to_lowercase();
                static UUID: OnceLock<Regex> = OnceLock::new();
                static DATE: OnceLock<Regex> = OnceLock::new();
                let uuid = UUID.get_or_init(|| {
                    Regex::new(r"(?i)^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$")
                        .expect("static regex")
                });
                let date =
                    DATE.get_or_init(|| Regex::new(r"^\d{4}-\d{2}-\d{2}(?:[T ][0-9:.+-Zz]+)?$").expect("static regex"));
                if s.len() < 2
                    || k == "id"
                    || k == "uuid"
                    || k.ends_with("_id")
                    || k.ends_with("-id")
                    || key.ends_with("Id")
                    || key.ends_with("ID")
                    || uuid.is_match(s)
                    || date.is_match(s)
                    || lower.starts_with("http://")
                    || lower.starts_with("https://")
                    || (s.len() == 3 && s.bytes().all(|b| b.is_ascii_uppercase()))
                {
                    return;
                }
                if s.chars().any(|c| c.is_alphanumeric()) {
                    out.push(s.to_owned());
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    if let Ok(v) = serde_json::from_str::<Value>(data) {
        collect(&v, "", &mut out);
    }
    out.join(" ")
}
