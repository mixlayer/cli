// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

package cli

import (
	"bytes"
	"os"
	"strings"
	"testing"

	"github.com/mixlayer/cli/internal/cliutil"
)

func TestSetTokenReadsStdin(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("MIXLAYER_HOME", home)
	t.Setenv("MIXLAYER_API_KEY", "")
	const key = "mlk-test-stdin-key-1234567890"
	root := RootCmd()
	var out bytes.Buffer
	root.SetOut(&out)
	root.SetErr(&out)
	root.SetIn(strings.NewReader(key + "\n"))
	root.SetArgs([]string{"auth", "set-token"})
	if err := root.Execute(); err != nil {
		t.Fatalf("set-token: %v\n%s", err, out.String())
	}
	if strings.Contains(out.String(), key) {
		t.Fatal("set-token printed the key")
	}
	path, err := cliutil.CredentialsFilePath()
	if err != nil {
		t.Fatal(err)
	}
	b, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(b), key) {
		t.Fatalf("credentials file does not contain the key: %s", path)
	}
	if fi, _ := os.Stat(path); fi.Mode().Perm() != 0o600 {
		t.Fatalf("credentials file mode %v, want 0600", fi.Mode().Perm())
	}
}

func TestSetTokenRejectsSpaces(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("MIXLAYER_HOME", home)
	root := RootCmd()
	root.SetOut(&bytes.Buffer{})
	root.SetErr(&bytes.Buffer{})
	root.SetIn(strings.NewReader("this is a sentence not a key"))
	root.SetArgs([]string{"auth", "set-token"})
	if err := root.Execute(); err == nil || ExitCode(err) != 2 {
		t.Fatalf("want usage error, got %v", err)
	}
}
