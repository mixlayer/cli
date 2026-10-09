// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

package cli

import (
	"encoding/csv"
	"fmt"
	"strings"

	"github.com/mixlayer/cli/internal/store"
)

// csvColumnTerms reads data as CSV and returns one run-scoped mask term for
// every non-empty cell in the named columns. Column names match the header
// row case-insensitively. Each term's kind is the column name in capitals
// (letters and underscores only), so a "company" column becomes COMPANY_<id>
// placeholders. Values shorter than two characters are skipped because they
// would mask unrelated text.
func csvColumnTerms(data string, columns []string) ([]store.DictionaryTerm, error) {
	if len(columns) == 0 {
		return nil, nil
	}
	if strings.TrimSpace(data) == "" {
		return nil, usageErr(fmt.Errorf("--mask-columns needs CSV data (pass a file with --data)"))
	}
	r := csv.NewReader(strings.NewReader(data))
	r.FieldsPerRecord = -1
	r.LazyQuotes = true
	r.TrimLeadingSpace = true
	rows, err := r.ReadAll()
	if err != nil {
		return nil, usageErr(fmt.Errorf("--mask-columns: data is not valid CSV: %w", err))
	}
	if len(rows) == 0 {
		return nil, usageErr(fmt.Errorf("--mask-columns: CSV has no header row"))
	}
	header := map[string]int{}
	for i, h := range rows[0] {
		header[strings.ToLower(strings.TrimSpace(strings.TrimPrefix(h, "\ufeff")))] = i
	}
	seen := map[string]bool{}
	var terms []store.DictionaryTerm
	for _, col := range columns {
		col = strings.TrimSpace(col)
		if col == "" {
			continue
		}
		idx, ok := header[strings.ToLower(col)]
		if !ok {
			return nil, usageErr(fmt.Errorf("--mask-columns: no column %q; the CSV has: %s", col, strings.Join(rows[0], ", ")))
		}
		kind := columnKind(col)
		for _, row := range rows[1:] {
			if idx >= len(row) {
				continue
			}
			v := strings.TrimSpace(row[idx])
			if len([]rune(v)) < 2 || seen[strings.ToLower(v)] {
				continue
			}
			seen[strings.ToLower(v)] = true
			terms = append(terms, store.DictionaryTerm{Term: v, Kind: kind})
		}
	}
	return terms, nil
}

// columnKind turns a column name into a placeholder kind the rehydrator
// recognises: capital letters joined by single underscores.
func columnKind(col string) string {
	var b strings.Builder
	lastUnderscore := true
	for _, r := range strings.ToUpper(col) {
		if r >= 'A' && r <= 'Z' {
			b.WriteRune(r)
			lastUnderscore = false
		} else if !lastUnderscore {
			b.WriteByte('_')
			lastUnderscore = true
		}
	}
	kind := strings.Trim(b.String(), "_")
	if kind == "" {
		return "FIELD"
	}
	return kind
}
