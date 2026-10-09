// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.
#![allow(dead_code)] // Store APIs are wired to commands in a later milestone.

mod schema;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, NaiveDateTime, Utc};
use regex::Regex;
use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params, types::ValueRef};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::shield::{DictionaryTerm, Vault, VaultEntry, value_hash};

pub use schema::VERSION as STORE_SCHEMA_VERSION;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RunRecord {
    pub id: String,
    pub group_id: String,
    pub command: String,
    pub prompt: String,
    pub answer: String,
    pub reasoning: String,
    pub model: String,
    pub seed: i64,
    pub params_json: Option<Value>,
    pub raw_json: Option<Value>,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
    pub cost_usd: f64,
    pub cost_unknown: bool,
    pub catalog_source: String,
    pub latency_ms: i64,
    pub created_at: Option<DateTime<Utc>>,
}
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AuditRecord {
    pub id: String,
    pub run_id: String,
    pub command: String,
    pub payload_sha256: String,
    pub byte_count: i64,
    pub model: String,
    pub guard_model: String,
    pub masked_entities: i64,
    pub leaked_pii_count: i64,
    pub cost_usd: Option<f64>,
    pub upstream_host: String,
    pub detector: String,
    pub detector_host: String,
    pub detector_fallback: String,
    pub detector_notice: String,
    pub tripwire_detector: String,
    pub ignored_model_spans: i64,
    pub detector_cost_usd: Option<f64>,
    pub masked_counts: Option<HashMap<String, i64>>,
    pub restored_count: i64,
    pub unrestored_count: i64,
    pub unrestored_tokens: Option<Vec<String>>,
    pub repairs: Option<Vec<String>>,
    pub created_at: Option<DateTime<Utc>>,
}
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ModelCacheRecord {
    pub id: String,
    pub context_window: i64,
    pub supports_tools: bool,
    pub supports_reasoning: bool,
    pub input_price_per_million: f64,
    pub output_price_per_million: f64,
    pub raw_json: Option<Value>,
    pub updated_at: Option<DateTime<Utc>>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct StoredVaultEntry {
    pub token: String,
    pub value: String,
    pub kind: String,
    pub created_at: Option<DateTime<Utc>>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct LadderRecord {
    pub id: String,
    pub prompt: String,
    pub rungs: Vec<String>,
    pub first_confident_model: String,
    pub judge_model: String,
    pub created_at: Option<DateTime<Utc>>,
}

pub struct Store {
    conn: Connection,
    path: PathBuf,
}

impl Store {
    pub fn open_default() -> Result<Self> {
        Self::open(crate::paths::dir(crate::paths::Kind::Data)?.join("data.db"))
    }
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            let mut builder = fs::DirBuilder::new();
            builder.recursive(true).mode(0o700).create(parent)?;
        }
        #[cfg(not(unix))]
        fs::create_dir_all(parent)?;
        let _ = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .or_else(|e| {
                if e.kind() == std::io::ErrorKind::AlreadyExists {
                    Ok(std::fs::File::open(path)?)
                } else {
                    Err(e)
                }
            })?;
        secure_files(path)?;
        let mut conn = Connection::open(path).context("opening database")?;
        conn.busy_timeout(Duration::from_millis(5000))?;
        let current = schema::version(&conn)?;
        if current > STORE_SCHEMA_VERSION {
            bail!(
                "database schema version {current} is newer than supported version {STORE_SCHEMA_VERSION}; upgrade the CLI binary or open an older database"
            );
        }
        // Several processes opening a fresh database at once race on the
        // WAL conversion and the migration lock; SQLite can return BUSY
        // there before the busy handler applies. Retry like the Go store
        // (30s budget, 5–100ms backoff).
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        retry_on_busy(deadline, || conn.pragma_update(None, "journal_mode", "WAL"))?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "temp_store", "MEMORY")?;
        conn.pragma_update(None, "mmap_size", 268435456_i64)?;
        retry_on_busy(deadline, || schema::migrate(&mut conn)).context("running migrations")?;
        secure_files(path)?;
        Ok(Self {
            conn,
            path: path.to_owned(),
        })
    }
    pub fn open_read_only(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI)?;
        conn.busy_timeout(Duration::from_millis(5000))?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "temp_store", "MEMORY")?;
        conn.pragma_update(None, "mmap_size", 268435456_i64)?;
        Ok(Self {
            conn,
            path: path.to_owned(),
        })
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn connection(&self) -> &Connection {
        &self.conn
    }
    pub fn schema_version(&self) -> Result<i64> {
        schema::version(&self.conn)
    }

    pub fn save_run(&mut self, mut r: RunRecord) -> Result<()> {
        if r.id.is_empty() {
            r.id = new_id("run")?;
        }
        let created = r
            .created_at
            .unwrap_or_else(Utc::now)
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let tx = self.conn.transaction()?;
        tx.execute("INSERT INTO runs (id, group_id, command, prompt, answer, reasoning, model, seed, params_json, raw_json, prompt_tokens, completion_tokens, total_tokens, cost_usd, latency_ms, created_at) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
            params![r.id, null_empty(&r.group_id), r.command, r.prompt, r.answer, r.reasoning, r.model,
                (r.seed != 0).then_some(r.seed), json_text(&r.params_json)?, json_text(&r.raw_json)?, r.prompt_tokens,
                r.completion_tokens, r.total_tokens, (!r.cost_unknown).then_some(r.cost_usd), r.latency_ms, created])?;
        tx.execute(
            "INSERT INTO runs_fts (id,prompt,answer,reasoning) VALUES (?,?,?,?)",
            params![r.id, r.prompt, r.answer, r.reasoning],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn get_run(&self, id: &str) -> Result<Option<RunRecord>> {
        let mut stmt = self.conn.prepare("SELECT id,group_id,command,prompt,answer,reasoning,model,seed,params_json,raw_json,prompt_tokens,completion_tokens,total_tokens,cost_usd,latency_ms,created_at FROM runs WHERE id=?")?;
        let row = stmt
            .query_row([id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, Option<String>>(5)?,
                    r.get::<_, Option<String>>(6)?,
                    r.get::<_, Option<i64>>(7)?,
                    r.get::<_, Option<String>>(8)?,
                    r.get::<_, Option<String>>(9)?,
                    r.get::<_, i64>(10)?,
                    r.get::<_, i64>(11)?,
                    r.get::<_, i64>(12)?,
                    r.get::<_, Option<f64>>(13)?,
                    r.get::<_, i64>(14)?,
                    r.get::<_, String>(15)?,
                ))
            })
            .optional()?;
        row.map(
            |(
                id,
                group,
                command,
                prompt,
                answer,
                reasoning,
                model,
                seed,
                params,
                raw,
                pt,
                ct,
                tt,
                cost,
                latency,
                created,
            )| {
                Ok(RunRecord {
                    id,
                    group_id: group.unwrap_or_default(),
                    command,
                    prompt,
                    answer: answer.unwrap_or_default(),
                    reasoning: reasoning.unwrap_or_default(),
                    model: model.unwrap_or_default(),
                    seed: seed.unwrap_or_default(),
                    params_json: parse_json(params)?,
                    raw_json: parse_json(raw)?,
                    prompt_tokens: pt,
                    completion_tokens: ct,
                    total_tokens: tt,
                    cost_usd: cost.unwrap_or_default(),
                    cost_unknown: cost.is_none(),
                    latency_ms: latency,
                    created_at: parse_stored_time(&created),
                    ..Default::default()
                })
            },
        )
        .transpose()
    }
    pub fn recent_runs(&self, limit: i64) -> Result<Vec<RunRecord>> {
        self.run_ids(
            "SELECT id FROM runs ORDER BY created_at DESC LIMIT ?",
            if limit <= 0 { 20 } else { limit },
        )
    }
    pub fn search_runs(&self, query: &str, limit: i64) -> Result<Vec<RunRecord>> {
        let limit = if limit <= 0 { 20 } else { limit };
        let safe = safe_fts_query(query);
        let mut stmt = self.conn.prepare(
            "SELECT r.id FROM runs_fts f JOIN runs r ON r.id=f.id WHERE runs_fts MATCH ? ORDER BY rank LIMIT ?",
        )?;
        let ids: Vec<String> = stmt
            .query_map(params![safe, limit], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        ids.into_iter()
            .map(|id| self.get_run(&id)?.ok_or_else(|| anyhow!("indexed run missing: {id}")))
            .collect()
    }
    fn run_ids(&self, sql: &str, limit: i64) -> Result<Vec<RunRecord>> {
        let mut stmt = self.conn.prepare(sql)?;
        let ids: Vec<String> = stmt
            .query_map([limit], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        ids.into_iter()
            .map(|id| self.get_run(&id)?.ok_or_else(|| anyhow!("run missing: {id}")))
            .collect()
    }

    pub fn token_for_value(&self, value: &str) -> Result<Option<StoredVaultEntry>> {
        vault_by_hash(&self.conn, &value_hash(value))
    }
    pub fn ensure_vault_entry(&mut self, kind: &str, value: &str) -> Result<StoredVaultEntry> {
        let hash = value_hash(value);
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(e) = vault_by_hash(&tx, &hash)? {
            tx.commit()?;
            return Ok(e);
        }
        let mut token = None;
        for _ in 0..10 {
            let mut suffix = [0_u8; 4];
            getrandom::fill(&mut suffix).map_err(|e| anyhow!("vault entropy: {e}"))?;
            let candidate = format!("{kind}_{}", hex::encode(suffix));
            let exists = tx
                .query_row("SELECT 1 FROM vault WHERE token=?", [&candidate], |_| Ok(()))
                .optional()?;
            if exists.is_none() {
                token = Some(candidate);
                break;
            }
        }
        let token = token.ok_or_else(|| anyhow!("could not allocate unique vault token for {kind}"))?;
        let created = Utc::now();
        tx.execute(
            "INSERT INTO vault (token,value,kind,value_hash,created_at) VALUES (?,?,?,?,?)",
            params![
                token,
                value,
                kind,
                hash,
                created.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
            ],
        )?;
        tx.commit()?;
        Ok(StoredVaultEntry {
            token,
            value: value.into(),
            kind: kind.into(),
            created_at: Some(created),
        })
    }
    pub fn save_vault_entry(&mut self, e: &StoredVaultEntry) -> Result<()> {
        self.conn.execute("INSERT INTO vault (token,value,kind,value_hash,created_at) VALUES (?,?,?,?,?) ON CONFLICT(value_hash) DO UPDATE SET token=excluded.token,kind=excluded.kind",
            params![e.token,e.value,e.kind,value_hash(&e.value),e.created_at.unwrap_or_else(Utc::now).to_rfc3339_opts(chrono::SecondsFormat::Secs,true)])?;
        Ok(())
    }
    pub fn vault_entries(&self, reveal: bool) -> Result<Vec<StoredVaultEntry>> {
        let mut stmt = self
            .conn
            .prepare("SELECT token,value,kind,created_at FROM vault ORDER BY kind,token")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?;
        rows.map(|r| {
            let (token, value, kind, created) = r?;
            Ok(StoredVaultEntry {
                token,
                value: if reveal { value } else { String::new() },
                kind,
                created_at: parse_stored_time(&created),
            })
        })
        .collect()
    }
    pub fn vault_token_map(&self) -> Result<HashMap<String, String>> {
        let mut stmt = self.conn.prepare("SELECT token,value FROM vault")?;
        Ok(stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?)
    }
    pub fn purge_vault(&mut self) -> Result<usize> {
        Ok(self.conn.execute("DELETE FROM vault", [])?)
    }
    pub fn vault_rotation_status(&self) -> (bool, &'static str) {
        (false, "rotation is deferred to preserve existing audit receipts")
    }

    pub fn add_dictionary_term(&mut self, term: &str, kind: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO shield_dictionary(term,kind) VALUES(?,?) ON CONFLICT(term) DO UPDATE SET kind=excluded.kind",
            params![term.trim(), kind],
        )?;
        Ok(())
    }
    pub fn dictionary_terms(&self) -> Result<Vec<DictionaryTerm>> {
        let mut stmt = self
            .conn
            .prepare("SELECT term,kind FROM shield_dictionary ORDER BY term COLLATE NOCASE")?;
        Ok(stmt
            .query_map([], |r| {
                Ok(DictionaryTerm {
                    term: r.get(0)?,
                    kind: r.get(1)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }
    pub fn remove_dictionary_term(&mut self, term: &str) -> Result<bool> {
        Ok(self
            .conn
            .execute("DELETE FROM shield_dictionary WHERE term=?", [term])?
            > 0)
    }

    pub fn save_audit(&mut self, mut a: AuditRecord) -> Result<()> {
        if a.id.is_empty() {
            a.id = new_id("audit")?;
        }
        let created = a
            .created_at
            .unwrap_or_else(Utc::now)
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        self.conn.execute("INSERT INTO audit (id,run_id,command,payload_sha256,byte_count,model,guard_model,masked_entities,leaked_pii_count,cost_usd,created_at,upstream_host,detector,detector_host,detector_fallback,detector_notice,tripwire_detector,ignored_model_spans,detector_cost_usd,masked_counts,restored_count,unrestored_count,unrestored_tokens,repairs) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
            params![a.id,null_empty(&a.run_id),a.command,a.payload_sha256,a.byte_count,null_empty(&a.model),null_empty(&a.guard_model),a.masked_entities,a.leaked_pii_count,a.cost_usd,created,null_empty(&a.upstream_host),null_empty(&a.detector),null_empty(&a.detector_host),null_empty(&a.detector_fallback),null_empty(&a.detector_notice),null_empty(&a.tripwire_detector),a.ignored_model_spans,a.detector_cost_usd,serde_json::to_string(&a.masked_counts)?,a.restored_count,a.unrestored_count,serde_json::to_string(&a.unrestored_tokens)?,serde_json::to_string(&a.repairs)?])?;
        Ok(())
    }
    pub fn audit_records(&self, id: &str, limit: i64) -> Result<Vec<AuditRecord>> {
        let sql = "SELECT id,run_id,command,payload_sha256,byte_count,model,guard_model,masked_entities,leaked_pii_count,cost_usd,created_at,upstream_host,detector,detector_host,detector_fallback,detector_notice,tripwire_detector,ignored_model_spans,detector_cost_usd,masked_counts,restored_count,unrestored_count,unrestored_tokens,repairs FROM audit";
        let query = if id.is_empty() {
            format!("{sql} ORDER BY created_at DESC LIMIT ?")
        } else {
            format!("{sql} WHERE id=? ORDER BY created_at DESC LIMIT ?")
        };
        let mut stmt = self.conn.prepare(&query)?;
        let limit = if limit <= 0 { 50 } else { limit };
        let scan = |r: &rusqlite::Row<'_>| -> rusqlite::Result<AuditTuple> {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
                r.get(7)?,
                r.get(8)?,
                r.get(9)?,
                r.get(10)?,
                r.get(11)?,
                r.get(12)?,
                r.get(13)?,
                r.get(14)?,
                r.get(15)?,
                r.get(16)?,
                r.get(17)?,
                r.get(18)?,
                r.get(19)?,
                r.get(20)?,
                r.get(21)?,
                r.get(22)?,
                r.get(23)?,
            ))
        };
        let rows: Vec<AuditTuple> = if id.is_empty() {
            stmt.query_map([limit], scan)?.collect::<rusqlite::Result<_>>()?
        } else {
            stmt.query_map(params![id, limit], scan)?
                .collect::<rusqlite::Result<_>>()?
        };
        rows.into_iter()
            .map(
                |(
                    id,
                    run_id,
                    command,
                    payload_sha256,
                    byte_count,
                    model,
                    guard_model,
                    masked_entities,
                    leaked_pii_count,
                    cost_usd,
                    created,
                    upstream_host,
                    detector,
                    detector_host,
                    detector_fallback,
                    detector_notice,
                    tripwire_detector,
                    ignored_model_spans,
                    detector_cost_usd,
                    counts,
                    restored_count,
                    unrestored_count,
                    tokens,
                    repairs,
                )| {
                    Ok(AuditRecord {
                        id,
                        run_id: run_id.unwrap_or_default(),
                        command,
                        payload_sha256,
                        byte_count,
                        model: model.unwrap_or_default(),
                        guard_model: guard_model.unwrap_or_default(),
                        masked_entities,
                        leaked_pii_count,
                        cost_usd,
                        upstream_host: upstream_host.unwrap_or_default(),
                        detector: detector.unwrap_or_default(),
                        detector_host: detector_host.unwrap_or_default(),
                        detector_fallback: detector_fallback.unwrap_or_default(),
                        detector_notice: detector_notice.unwrap_or_default(),
                        tripwire_detector: tripwire_detector.unwrap_or_default(),
                        ignored_model_spans,
                        detector_cost_usd,
                        masked_counts: parse_json_typed(counts)?,
                        restored_count,
                        unrestored_count,
                        unrestored_tokens: parse_json_typed(tokens)?,
                        repairs: parse_json_typed(repairs)?,
                        created_at: parse_stored_time(&created),
                    })
                },
            )
            .collect()
    }

    pub fn save_ladder(
        &mut self,
        id: &str,
        prompt: &str,
        rungs: &[String],
        first_confident: &str,
        judge: &str,
    ) -> Result<()> {
        let id = if id.is_empty() { new_id("ladder")? } else { id.into() };
        self.conn.execute(
            "INSERT INTO ladders (id,prompt,rungs,first_confident_model,judge_model,created_at) VALUES (?,?,?,?,?,?)",
            params![
                id,
                prompt,
                serde_json::to_string(rungs)?,
                null_empty(first_confident),
                null_empty(judge),
                Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
            ],
        )?;
        Ok(())
    }
    pub fn ladders(&self) -> Result<Vec<LadderRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id,prompt,rungs,first_confident_model,judge_model,created_at FROM ladders ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, String>(5)?,
            ))
        })?;
        rows.map(|r| {
            let (id, prompt, rungs, first, judge, created) = r?;
            Ok(LadderRecord {
                id,
                prompt,
                rungs: serde_json::from_str(&rungs)?,
                first_confident_model: first.unwrap_or_default(),
                judge_model: judge.unwrap_or_default(),
                created_at: parse_stored_time(&created),
            })
        })
        .collect()
    }
    pub fn save_model_cache(&mut self, m: &ModelCacheRecord) -> Result<()> {
        self.conn.execute("INSERT INTO models_cache (id,context_window,supports_tools,supports_reasoning,input_price_per_million,output_price_per_million,raw_json,updated_at) VALUES (?,?,?,?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET context_window=excluded.context_window,supports_tools=excluded.supports_tools,supports_reasoning=excluded.supports_reasoning,input_price_per_million=excluded.input_price_per_million,output_price_per_million=excluded.output_price_per_million,raw_json=excluded.raw_json,updated_at=excluded.updated_at",
            params![m.id,m.context_window,m.supports_tools,m.supports_reasoning,m.input_price_per_million,m.output_price_per_million,json_text(&m.raw_json)?,m.updated_at.unwrap_or_else(Utc::now).to_rfc3339_opts(chrono::SecondsFormat::Secs,true)])?;
        Ok(())
    }
    pub fn all_models(&self) -> Result<Vec<ModelCacheRecord>> {
        let mut stmt=self.conn.prepare("SELECT id,context_window,supports_tools,supports_reasoning,input_price_per_million,output_price_per_million,raw_json,updated_at FROM models_cache ORDER BY id")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, bool>(2)?,
                r.get::<_, bool>(3)?,
                r.get::<_, f64>(4)?,
                r.get::<_, f64>(5)?,
                r.get::<_, Option<String>>(6)?,
                r.get::<_, String>(7)?,
            ))
        })?;
        rows.map(|r| {
            let (
                id,
                context_window,
                supports_tools,
                supports_reasoning,
                input_price_per_million,
                output_price_per_million,
                raw,
                updated,
            ) = r?;
            Ok(ModelCacheRecord {
                id,
                context_window,
                supports_tools,
                supports_reasoning,
                input_price_per_million,
                output_price_per_million,
                raw_json: parse_json(raw)?,
                updated_at: parse_stored_time(&updated),
            })
        })
        .collect()
    }
    pub fn query_models(&self, dsl: &str) -> Result<Vec<ModelCacheRecord>> {
        let terms: Vec<String> = dsl.split_whitespace().map(str::to_lowercase).collect();
        Ok(self
            .all_models()?
            .into_iter()
            .filter(|m| terms.iter().all(|t| model_matches(m, t)))
            .collect())
    }
    pub fn save_live_catalog(&mut self, scope: &str, raw: &Value) -> Result<()> {
        self.conn.execute("INSERT INTO live_catalog_cache (scope,models_json,fetched_at) VALUES (?,?,?) ON CONFLICT(scope) DO UPDATE SET models_json=excluded.models_json,fetched_at=excluded.fetched_at",
            params![scope,serde_json::to_string(raw)?,Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Nanos,true)])?;
        Ok(())
    }
    pub fn cached_live_catalog(&self, scope: &str, max_age: Duration) -> Result<Option<Value>> {
        let row: Option<(String, String)> = self
            .conn
            .query_row(
                "SELECT models_json,fetched_at FROM live_catalog_cache WHERE scope=?",
                [scope],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        match row {
            Some((raw, fetched))
                if parse_stored_time(&fetched)
                    .is_some_and(|t| chrono::Duration::from_std(max_age).is_ok_and(|max| Utc::now() - t <= max)) =>
            {
                // The Go loader treats an undecodable catalog as a cache miss.
                Ok(serde_json::from_str(&raw).ok())
            }
            _ => Ok(None),
        }
    }
    pub fn query_read_only(&self, query: &str, private_tables_blocked: bool) -> Result<Vec<Map<String, Value>>> {
        validate_read_sql(query, private_tables_blocked)?;
        // The text check above is a fast first line. The authorizer is the
        // real boundary: SQLite reports every table a statement reads, also
        // through views and triggers, so private values stay unreadable
        // however the query is written.
        let _guard = private_tables_blocked.then(|| PrivateReadGuard::install(&self.conn));
        let mut stmt = self.conn.prepare(query).map_err(|e| {
            if e.to_string().contains("not authorized") || e.to_string().contains("prohibited") {
                anyhow!("the vault and shield dictionary hold private values and are not readable in agent mode")
            } else {
                e.into()
            }
        })?;
        if !stmt.readonly() {
            bail!("only SELECT-only read queries are allowed");
        }
        let columns: Vec<String> = stmt.column_names().iter().map(|s| (*s).to_owned()).collect();
        let mut rows = stmt.query([])?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            let mut object = Map::new();
            for (i, name) in columns.iter().enumerate() {
                let v = match row.get_ref(i)? {
                    ValueRef::Null => Value::Null,
                    ValueRef::Integer(n) => Value::from(n),
                    ValueRef::Real(n) => Value::from(n),
                    ValueRef::Text(s) | ValueRef::Blob(s) => Value::String(String::from_utf8_lossy(s).into_owned()),
                };
                object.insert(name.clone(), v);
            }
            out.push(object);
        }
        Ok(out)
    }
}

impl Vault for Store {
    fn ensure_entry(&mut self, kind: &str, value: &str) -> Result<VaultEntry> {
        let e = self.ensure_vault_entry(kind, value)?;
        Ok(VaultEntry {
            token: e.token,
            value: e.value,
            kind: e.kind,
        })
    }
    fn save_entry(&mut self, e: VaultEntry) -> Result<()> {
        self.save_vault_entry(&StoredVaultEntry {
            token: e.token,
            value: e.value,
            kind: e.kind,
            created_at: None,
        })
    }
    fn token_map(&self) -> Result<HashMap<String, String>> {
        self.vault_token_map()
    }
}

fn vault_by_hash(conn: &Connection, hash: &str) -> Result<Option<StoredVaultEntry>> {
    let row: Option<(String, String, String, String)> = conn
        .query_row(
            "SELECT token,value,kind,created_at FROM vault WHERE value_hash=?",
            [hash],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    Ok(row.map(|(token, value, kind, created)| StoredVaultEntry {
        token,
        value,
        kind,
        created_at: parse_stored_time(&created),
    }))
}
fn secure_files(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for file in [
            path.to_owned(),
            PathBuf::from(format!("{}-wal", path.display())),
            PathBuf::from(format!("{}-shm", path.display())),
        ] {
            if file.exists() {
                fs::set_permissions(file, fs::Permissions::from_mode(0o600))?;
            }
        }
    }
    #[cfg(not(unix))]
    let _ = path; // Windows has no POSIX modes; the per-user data dir is private.
    Ok(())
}
fn new_id(prefix: &str) -> Result<String> {
    let mut entropy = [0_u8; 16];
    getrandom::fill(&mut entropy).map_err(|e| anyhow!("ID entropy: {e}"))?;
    let digest = Sha256::digest(
        format!(
            "{prefix}-{}-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or_default(),
            hex::encode(entropy)
        )
        .as_bytes(),
    );
    Ok(format!("{prefix}_{}", hex::encode(&digest[..8])))
}
fn null_empty(s: &str) -> Option<&str> {
    if s.trim().is_empty() { None } else { Some(s) }
}
fn json_text(v: &Option<Value>) -> Result<Option<String>> {
    v.as_ref().map(serde_json::to_string).transpose().map_err(Into::into)
}
fn parse_json(raw: Option<String>) -> Result<Option<Value>> {
    raw.map(|s| serde_json::from_str(&s).map_err(Into::into)).transpose()
}
/// Decodes an optional JSON column. Like the Go store, a NULL, empty or
/// `null` value, or one that doesn't parse, reads as absent instead of
/// failing the whole row (older Go databases have such rows).
fn parse_json_typed<T: serde::de::DeserializeOwned>(raw: Option<String>) -> Result<Option<T>> {
    Ok(raw
        .filter(|s| !matches!(s.trim(), "" | "null"))
        .and_then(|s| serde_json::from_str(&s).ok()))
}

type AuditTuple = (
    String,
    Option<String>,
    String,
    String,
    i64,
    Option<String>,
    Option<String>,
    i64,
    i64,
    Option<f64>,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    i64,
    Option<f64>,
    Option<String>,
    i64,
    i64,
    Option<String>,
    Option<String>,
);

pub fn parse_stored_time(raw: &str) -> Option<DateTime<Utc>> {
    let s = raw.trim();
    if let Ok(t) = DateTime::parse_from_rfc3339(s) {
        return Some(t.with_timezone(&Utc));
    }
    // Go time.String(): 2026-01-02 03:04:05.123456789 -0600 CST m=+...
    let s = s.split(" m=").next().unwrap_or(s);
    for candidate in [Some(s), s.rsplit_once(' ').map(|(prefix, _)| prefix)]
        .into_iter()
        .flatten()
    {
        for fmt in ["%Y-%m-%d %H:%M:%S%.f %z", "%Y-%m-%d %H:%M:%S %z"] {
            if let Ok(t) = DateTime::parse_from_str(candidate, fmt) {
                return Some(t.with_timezone(&Utc));
            }
        }
    }
    NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
        .ok()
        .map(|t| t.and_utc())
}
fn safe_fts_query(query: &str) -> String {
    let tokens: Vec<String> = query
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
    if tokens.is_empty() {
        "\"\"".into()
    } else {
        tokens.join(" ")
    }
}
fn model_matches(m: &ModelCacheRecord, term: &str) -> bool {
    if term == "tools" {
        return m.supports_tools;
    }
    if term == "reasoning" {
        return m.supports_reasoning;
    }
    for (prefix, op) in [("ctx>=", 0), ("ctx>", 1), ("ctx=", 2)] {
        if let Some(raw) = term.strip_prefix(prefix) {
            let raw = raw.trim_end_matches('b');
            let (number, mul) = if let Some(n) = raw.strip_suffix('k') {
                (n, 1000)
            } else {
                (raw, 1)
            };
            let want = number.parse::<i64>().unwrap_or(0) * mul;
            return match op {
                0 => m.context_window >= want,
                1 => m.context_window > want,
                _ => m.context_window == want,
            };
        }
    }
    m.id.to_lowercase().contains(term)
}
/// Tables that agent-mode SQL and MCP must never read: the vault and the
/// shield dictionary hold original private values; the schema tables would
/// reveal view and trigger definitions.
pub const PRIVATE_TABLES: &[&str] = &[
    "vault",
    "shield_dictionary",
    "sqlite_master",
    "sqlite_schema",
    "sqlite_temp_master",
    "sqlite_temp_schema",
];

fn private_read_authorizer(ctx: rusqlite::hooks::AuthContext<'_>) -> rusqlite::hooks::Authorization {
    use rusqlite::hooks::{AuthAction, Authorization};
    match ctx.action {
        AuthAction::Read { table_name, .. } if PRIVATE_TABLES.iter().any(|t| t.eq_ignore_ascii_case(table_name)) => {
            Authorization::Deny
        }
        AuthAction::Pragma { .. } | AuthAction::Attach { .. } | AuthAction::Detach { .. } => Authorization::Deny,
        AuthAction::Function { function_name } if function_name.eq_ignore_ascii_case("load_extension") => {
            Authorization::Deny
        }
        _ => Authorization::Allow,
    }
}

/// Installs the private-table authorizer for one query and removes it on drop.
pub struct PrivateReadGuard<'c>(&'c Connection);

impl<'c> PrivateReadGuard<'c> {
    pub fn install(conn: &'c Connection) -> Self {
        conn.authorizer(Some(private_read_authorizer));
        Self(conn)
    }
}

impl Drop for PrivateReadGuard<'_> {
    fn drop(&mut self) {
        self.0
            .authorizer(None::<fn(rusqlite::hooks::AuthContext<'_>) -> rusqlite::hooks::Authorization>);
    }
}

pub(crate) fn validate_read_sql(query: &str, private_tables_blocked: bool) -> Result<()> {
    let trimmed = query.trim();
    if !Regex::new(r"(?i)^(select|with)\b")?.is_match(trimmed) {
        bail!("only SELECT-only read queries are allowed");
    }
    let stripped = strip_sql_quoted(trimmed);
    let forbidden = Regex::new(
        r"(?i)\b(insert|update|delete|drop|create|alter|replace|truncate|pragma|attach|detach|vacuum)\b|\bpragma_",
    )?;
    if forbidden.is_match(&stripped) {
        bail!("only SELECT-only read queries are allowed");
    }
    if private_tables_blocked {
        let private = Regex::new(
            r"(?i)(^|[^a-z0-9_])(vault|shield_dictionary|sqlite_master|sqlite_schema|pragma_[a-z_]+)([^a-z0-9_]|$)",
        )?;
        if private.is_match(query) {
            bail!("the vault and shield dictionary hold private values and are not readable in agent mode");
        }
    }
    Ok(())
}
fn strip_sql_quoted(query: &str) -> String {
    let bytes = query.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    let mut quote = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if quote != 0 {
            if b == quote {
                if i + 1 < bytes.len() && bytes[i + 1] == quote {
                    out.extend_from_slice(b"  ");
                    i += 2;
                    continue;
                }
                quote = 0;
            }
            out.push(b' ');
        } else if b == b'\'' || b == b'"' {
            quote = b;
            out.push(b' ');
        } else {
            out.push(b);
        }
        i += 1;
    }
    String::from_utf8(out).unwrap_or_default()
}

fn is_busy(e: &(dyn std::error::Error + 'static)) -> bool {
    let mut cur: Option<&(dyn std::error::Error + 'static)> = Some(e);
    while let Some(err) = cur {
        if let Some(rusqlite::Error::SqliteFailure(f, _)) = err.downcast_ref::<rusqlite::Error>()
            && matches!(
                f.code,
                rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
            )
        {
            return true;
        }
        cur = err.source();
    }
    false
}

/// Retries `op` while SQLite reports BUSY/LOCKED, with 5–100ms backoff, until
/// `deadline`.
fn retry_on_busy<T, E>(deadline: std::time::Instant, mut op: impl FnMut() -> std::result::Result<T, E>) -> Result<T>
where
    E: Into<anyhow::Error>,
{
    let mut backoff = Duration::from_millis(5);
    loop {
        match op() {
            Ok(v) => return Ok(v),
            Err(e) => {
                let e: anyhow::Error = e.into();
                if !is_busy(e.as_ref()) || std::time::Instant::now() >= deadline {
                    return Err(e);
                }
                std::thread::sleep(backoff);
                backoff = (backoff * 2).min(Duration::from_millis(100));
            }
        }
    }
}
