// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

package store

import (
	"context"
	"strings"
)

type DictionaryTerm struct {
	Term string `json:"term"`
	Kind string `json:"kind"`
}

func (s *Store) AddDictionaryTerm(ctx context.Context, term, kind string) error {
	s.writeMu.Lock()
	defer s.writeMu.Unlock()
	_, err := s.db.ExecContext(ctx, `INSERT INTO shield_dictionary(term,kind) VALUES(?,?) ON CONFLICT(term) DO UPDATE SET kind=excluded.kind`, strings.TrimSpace(term), kind)
	return err
}

func (s *Store) DictionaryTerms(ctx context.Context) ([]DictionaryTerm, error) {
	rows, err := s.db.QueryContext(ctx, `SELECT term,kind FROM shield_dictionary ORDER BY term COLLATE NOCASE`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var out []DictionaryTerm
	for rows.Next() {
		var term DictionaryTerm
		if err := rows.Scan(&term.Term, &term.Kind); err != nil {
			return nil, err
		}
		out = append(out, term)
	}
	return out, rows.Err()
}

func (s *Store) RemoveDictionaryTerm(ctx context.Context, term string) (bool, error) {
	s.writeMu.Lock()
	defer s.writeMu.Unlock()
	res, err := s.db.ExecContext(ctx, `DELETE FROM shield_dictionary WHERE term=?`, term)
	if err != nil {
		return false, err
	}
	n, err := res.RowsAffected()
	return n > 0, err
}
