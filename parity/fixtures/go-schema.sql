CREATE TABLE resources (
			id TEXT NOT NULL,
			resource_type TEXT NOT NULL,
			data JSON NOT NULL,
			synced_at DATETIME DEFAULT CURRENT_TIMESTAMP,
			updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
			PRIMARY KEY (resource_type, id)
		);
CREATE INDEX idx_resources_type ON resources(resource_type);
CREATE INDEX idx_resources_synced ON resources(synced_at);
CREATE TABLE sync_state (
			resource_type TEXT PRIMARY KEY,
			last_cursor TEXT,
			last_synced_at DATETIME,
			total_count INTEGER DEFAULT 0
		);
CREATE TABLE runs (
			id TEXT PRIMARY KEY,
			group_id TEXT,
			command TEXT NOT NULL,
			prompt TEXT NOT NULL,
			answer TEXT,
			reasoning TEXT,
			model TEXT,
			seed INTEGER,
			params_json JSON,
			raw_json JSON,
			prompt_tokens INTEGER DEFAULT 0,
			completion_tokens INTEGER DEFAULT 0,
			total_tokens INTEGER DEFAULT 0,
			cost_usd REAL DEFAULT 0,
			latency_ms INTEGER DEFAULT 0,
			created_at DATETIME DEFAULT CURRENT_TIMESTAMP
		);
CREATE VIRTUAL TABLE runs_fts USING fts5(
			id UNINDEXED, prompt, answer, reasoning, tokenize='porter unicode61'
		)
/* runs_fts(id,prompt,answer,reasoning) */;
CREATE TABLE 'runs_fts_data'(id INTEGER PRIMARY KEY, block BLOB);
CREATE TABLE 'runs_fts_idx'(segid, term, pgno, PRIMARY KEY(segid, term)) WITHOUT ROWID;
CREATE TABLE 'runs_fts_content'(id INTEGER PRIMARY KEY, c0, c1, c2, c3);
CREATE TABLE 'runs_fts_docsize'(id INTEGER PRIMARY KEY, sz BLOB);
CREATE TABLE 'runs_fts_config'(k PRIMARY KEY, v) WITHOUT ROWID;
CREATE INDEX idx_runs_created ON runs(created_at);
CREATE INDEX idx_runs_group ON runs(group_id);
CREATE TABLE vault (
			token TEXT PRIMARY KEY,
			value TEXT NOT NULL,
			kind TEXT NOT NULL,
			value_hash TEXT NOT NULL UNIQUE,
			created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
			rotated_at DATETIME
		);
CREATE INDEX idx_vault_kind ON vault(kind);
CREATE TABLE audit (
			id TEXT PRIMARY KEY,
			run_id TEXT,
			command TEXT NOT NULL,
			payload_sha256 TEXT NOT NULL,
			byte_count INTEGER NOT NULL,
			model TEXT,
			guard_model TEXT,
			masked_entities INTEGER DEFAULT 0,
			leaked_pii_count INTEGER DEFAULT 0,
			cost_usd REAL DEFAULT 0,
			created_at DATETIME DEFAULT CURRENT_TIMESTAMP
		, "upstream_host" TEXT, "detector" TEXT, "detector_host" TEXT, "detector_fallback" TEXT, "detector_notice" TEXT, "tripwire_detector" TEXT, "ignored_model_spans" INTEGER DEFAULT 0, "detector_cost_usd" REAL, "masked_counts" TEXT, "restored_count" INTEGER DEFAULT 0, "unrestored_count" INTEGER DEFAULT 0, "unrestored_tokens" TEXT, "repairs" TEXT);
CREATE INDEX idx_audit_created ON audit(created_at);
CREATE TABLE shield_dictionary (term TEXT PRIMARY KEY COLLATE NOCASE, kind TEXT NOT NULL);
CREATE TABLE ladders (
			id TEXT PRIMARY KEY,
			prompt TEXT NOT NULL,
			rungs TEXT NOT NULL,
			first_confident_model TEXT,
			judge_model TEXT,
			created_at DATETIME DEFAULT CURRENT_TIMESTAMP
		);
CREATE TABLE models_cache (
			id TEXT PRIMARY KEY,
			context_window INTEGER DEFAULT 131072,
			supports_tools BOOLEAN DEFAULT TRUE,
			supports_reasoning BOOLEAN DEFAULT TRUE,
			input_price_per_million REAL DEFAULT 0,
			output_price_per_million REAL DEFAULT 0,
			raw_json JSON,
			updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
		);
CREATE TABLE live_catalog_cache (
			scope TEXT PRIMARY KEY,
			models_json TEXT NOT NULL,
			fetched_at DATETIME NOT NULL
		);
CREATE VIRTUAL TABLE resources_fts USING fts5(
	id, resource_type, content, tokenize='porter unicode61'
)
/* resources_fts(id,resource_type,content) */;
CREATE TABLE 'resources_fts_data'(id INTEGER PRIMARY KEY, block BLOB);
CREATE TABLE 'resources_fts_idx'(segid, term, pgno, PRIMARY KEY(segid, term)) WITHOUT ROWID;
CREATE TABLE 'resources_fts_content'(id INTEGER PRIMARY KEY, c0, c1, c2);
CREATE TABLE 'resources_fts_docsize'(id INTEGER PRIMARY KEY, sz BLOB);
CREATE TABLE 'resources_fts_config'(k PRIMARY KEY, v) WITHOUT ROWID;
