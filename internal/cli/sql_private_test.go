// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

package cli

import (
	"bytes"
	"path/filepath"
	"testing"
)

func TestSQLAgentModeCannotReadVault(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("MIXLAYER_HOME", home)
	db := filepath.Join(home, "ledger.db")
	s, err := openMixStore(t.Context(), db)
	if err != nil {
		t.Fatal(err)
	}
	s.Close()
	for _, q := range []string{"select value from vault", "select term from shield_dictionary"} {
		root := RootCmd()
		root.SetOut(&bytes.Buffer{})
		root.SetErr(&bytes.Buffer{})
		root.SetArgs([]string{"sql", q, "--db", db, "--agent"})
		if err := root.Execute(); err == nil || ExitCode(err) != 2 {
			t.Errorf("%q in agent mode: want usage error, got %v", q, err)
		}
	}
	root := RootCmd()
	root.SetOut(&bytes.Buffer{})
	root.SetErr(&bytes.Buffer{})
	root.SetArgs([]string{"sql", "select count(*) from runs", "--db", db, "--agent"})
	if err := root.Execute(); err != nil {
		t.Fatalf("ledger query in agent mode: %v", err)
	}
}
