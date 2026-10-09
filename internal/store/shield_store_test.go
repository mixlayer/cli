package store

import (
	"context"
	"database/sql"
	"path/filepath"
	"testing"
)

func TestAuditMigrationPreservesExistingRows(t *testing.T) {
	path := filepath.Join(t.TempDir(), "legacy.db")
	db, err := sql.Open("sqlite", path)
	if err != nil {
		t.Fatal(err)
	}
	_, err = db.Exec(`CREATE TABLE audit (id TEXT PRIMARY KEY,run_id TEXT,command TEXT NOT NULL,payload_sha256 TEXT NOT NULL,byte_count INTEGER NOT NULL,model TEXT,guard_model TEXT,masked_entities INTEGER DEFAULT 0,leaked_pii_count INTEGER DEFAULT 0,cost_usd REAL DEFAULT 0,created_at DATETIME DEFAULT CURRENT_TIMESTAMP);
		INSERT INTO audit(id,command,payload_sha256,byte_count,model,masked_entities,leaked_pii_count,cost_usd,created_at)
		VALUES('old','shield ask','abc',10,'legacy',1,0,0.01,'2026-01-01T00:00:00Z'); PRAGMA user_version=4;`)
	if err != nil {
		t.Fatal(err)
	}
	db.Close()
	s, err := Open(path)
	if err != nil {
		t.Fatal(err)
	}
	defer s.Close()
	rows, err := s.AuditRecords(context.Background(), "old", 1)
	if err != nil {
		t.Fatal(err)
	}
	if len(rows) != 1 || rows[0].Model != "legacy" || rows[0].CostUSD == nil || *rows[0].CostUSD != 0.01 {
		t.Fatalf("legacy audit = %#v", rows)
	}
	if err := s.SaveAudit(context.Background(), AuditRecord{ID: "new", Command: "shield ask", PayloadSHA256: "def", ByteCount: 5, UpstreamHost: "api.example.test", MaskedCounts: map[string]int{"PERSON": 1}}); err != nil {
		t.Fatal(err)
	}
	rows, err = s.AuditRecords(context.Background(), "new", 1)
	if err != nil {
		t.Fatal(err)
	}
	if len(rows) != 1 || rows[0].UpstreamHost != "api.example.test" || rows[0].MaskedCounts["PERSON"] != 1 || rows[0].CostUSD != nil {
		t.Fatalf("new audit = %#v", rows)
	}
}
