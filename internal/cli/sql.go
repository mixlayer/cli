// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

package cli

import (
	"database/sql"
	"fmt"
	"regexp"
	"strings"

	"github.com/mixlayer/cli/internal/store"
	"github.com/spf13/cobra"
)

// cli:data-source local
func newSQLCmd(flags *rootFlags) *cobra.Command {
	var dbPath string
	cmd := &cobra.Command{
		Use:         "sql <query>",
		Short:       "Run read-only SQL against the local reasoning ledger",
		Example:     `  mixlayer sql "select model, count(*) from runs group by model" --json`,
		Annotations: map[string]string{"mcp:read-only": "true"},
		RunE: func(cmd *cobra.Command, args []string) error {
			if len(args) == 0 {
				return cmd.Help()
			}
			query := strings.TrimSpace(args[0])
			if err := validateReadOnlySQL(query); err != nil {
				return err
			}
			// Agents relay output to remote models, so agent-mode runs may not
			// read the tables that hold original private values.
			if (flags.agent || shouldDetectAgent()) && privateSQLTableRe.MatchString(query) {
				return usageErr(fmt.Errorf("the vault and shield dictionary hold private values and are not readable in agent mode; use 'mixlayer vault rehydrate' locally"))
			}
			if dbPath == "" {
				dbPath = defaultDBPath("mixlayer")
			}
			s, err := store.OpenReadOnlyContext(cmd.Context(), dbPath)
			if err != nil {
				return err
			}
			defer s.Close()
			rows, err := s.DB().QueryContext(cmd.Context(), query)
			if err != nil {
				return err
			}
			defer rows.Close()
			out, err := rowsToMaps(rows)
			if err != nil {
				return err
			}
			return outputJSON(cmd, out)
		},
	}
	cmd.Flags().StringVar(&dbPath, "db", "", "SQLite database file path")
	return cmd
}

var privateSQLTableRe = regexp.MustCompile(`(?i)(^|[^a-z0-9_])(vault|shield_dictionary|sqlite_master|sqlite_schema|pragma_[a-z_]+)([^a-z0-9_]|$)`)

func validateReadOnlySQL(query string) error {
	trimmed := strings.TrimSpace(query)
	if !allowedReadSQLPrefixRe.MatchString(trimmed) {
		return usageErr(fmt.Errorf("only SELECT-only read queries are allowed"))
	}
	if forbiddenSQLKeywordRe.MatchString(stripSQLQuotedText(trimmed)) {
		return usageErr(fmt.Errorf("only SELECT-only read queries are allowed"))
	}
	return nil
}

func stripSQLQuotedText(query string) string {
	var b strings.Builder
	inSingle := false
	inDouble := false
	for i := 0; i < len(query); i++ {
		ch := query[i]
		switch {
		case inSingle:
			if ch == '\'' {
				if i+1 < len(query) && query[i+1] == '\'' {
					i++
					continue
				}
				inSingle = false
			}
			b.WriteByte(' ')
		case inDouble:
			if ch == '"' {
				if i+1 < len(query) && query[i+1] == '"' {
					i++
					continue
				}
				inDouble = false
			}
			b.WriteByte(' ')
		case ch == '\'':
			inSingle = true
			b.WriteByte(' ')
		case ch == '"':
			inDouble = true
			b.WriteByte(' ')
		default:
			b.WriteByte(ch)
		}
	}
	return b.String()
}

var (
	allowedReadSQLPrefixRe = regexp.MustCompile(`(?i)^(select|with)\b`)
	forbiddenSQLKeywordRe  = regexp.MustCompile(`(?i)\b(insert|update|delete|drop|create|alter|replace|truncate|pragma|attach|detach|vacuum)\b|\bpragma_`)
)

func rowsToMaps(rows *sql.Rows) ([]map[string]any, error) {
	cols, err := rows.Columns()
	if err != nil {
		return nil, err
	}
	var out []map[string]any
	for rows.Next() {
		values := make([]any, len(cols))
		ptrs := make([]any, len(cols))
		for i := range values {
			ptrs[i] = &values[i]
		}
		if err := rows.Scan(ptrs...); err != nil {
			return nil, err
		}
		row := map[string]any{}
		for i, c := range cols {
			switch v := values[i].(type) {
			case []byte:
				row[c] = string(v)
			default:
				row[c] = v
			}
		}
		out = append(out, row)
	}
	return out, rows.Err()
}
