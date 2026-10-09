package store

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"time"
)

func (s *Store) CachedLiveCatalog(ctx context.Context, scope string, maxAge time.Duration) (json.RawMessage, bool, error) {
	var raw, fetched string
	err := s.db.QueryRowContext(ctx, `SELECT models_json, fetched_at FROM live_catalog_cache WHERE scope = ?`, scope).Scan(&raw, &fetched)
	if errors.Is(err, sql.ErrNoRows) {
		return nil, false, nil
	}
	if err != nil {
		return nil, false, err
	}
	when, err := time.Parse(time.RFC3339Nano, fetched)
	if err != nil || time.Since(when) > maxAge {
		return nil, false, nil
	}
	return json.RawMessage(raw), true, nil
}

func (s *Store) SaveLiveCatalog(ctx context.Context, scope string, raw json.RawMessage) error {
	s.writeMu.Lock()
	defer s.writeMu.Unlock()
	_, err := s.db.ExecContext(ctx, `INSERT INTO live_catalog_cache (scope, models_json, fetched_at) VALUES (?, ?, ?)
		ON CONFLICT(scope) DO UPDATE SET models_json = excluded.models_json, fetched_at = excluded.fetched_at`, scope, string(raw), time.Now().UTC().Format(time.RFC3339Nano))
	return err
}
