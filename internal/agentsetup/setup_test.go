// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.
package agentsetup

import (
	"bytes"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestSetupEachAgentDryRunInstallIdempotentUninstall(t *testing.T) {
	for _, agent := range order {
		t.Run(agent, func(t *testing.T) {
			home := t.TempDir()
			t.Setenv("HOME", home)
			t.Setenv("CLAUDE_CONFIG_DIR", filepath.Join(home, ".claude"))
			key := "synthetic-secret-never-write-8472"
			t.Setenv("MIXLAYER_API_KEY", key)
			fakeDir := t.TempDir()
			fakeClaude := filepath.Join(fakeDir, "claude")
			stub := "#!/bin/sh\nconfig_dir=${CLAUDE_CONFIG_DIR:-$HOME}\nmkdir -p \"$config_dir\"\nif [ \"$2\" = add ]; then printf '{\"mcpServers\":{\"mixlayer\":{\"command\":\"fake\"}}}\\n' > \"$config_dir/.claude.json\"; else printf '{}\\n' > \"$config_dir/.claude.json\"; fi\n"
			if err := os.WriteFile(fakeClaude, []byte(stub), 0o700); err != nil {
				t.Fatal(err)
			}
			t.Setenv("PATH", fakeDir+string(os.PathListSeparator)+os.Getenv("PATH"))
			mcp := filepath.Join(fakeDir, "mixlayer-mcp")
			if err := os.WriteFile(mcp, []byte("#!/bin/sh\nexit 0\n"), 0o700); err != nil {
				t.Fatal(err)
			}
			opts := Options{Agent: agent, MCPBinary: mcp, DryRun: true}
			preview, err := Run(opts)
			if err != nil {
				t.Fatal(err)
			}
			if !preview.DryRun || len(preview.Actions) == 0 {
				t.Fatalf("bad preview: %+v", preview)
			}
			if names, err := os.ReadDir(home); err != nil || len(names) != 0 {
				t.Fatalf("dry-run wrote to HOME: %v, %v", names, err)
			}
			if agent == "claude-code" {
				configPath := filepath.Join(home, ".claude", ".claude.json")
				if err := os.MkdirAll(filepath.Dir(configPath), 0o700); err != nil {
					t.Fatal(err)
				}
				if err := os.WriteFile(configPath, []byte("{}\n"), 0o600); err != nil {
					t.Fatal(err)
				}
			}
			opts.DryRun = false
			installed, err := Run(opts)
			if err != nil {
				t.Fatal(err)
			}
			assertNoKey(t, home, key)
			if agent != "claude-desktop" {
				skill := filepath.Join(home, agents[agent], "SKILL.md")
				if _, err := os.Stat(skill); err != nil {
					t.Fatalf("skill not installed: %v", err)
				}
			}
			if agent != "hermes" {
				if !hasStatus(installed, "registered") {
					t.Fatalf("MCP not registered: %+v", installed)
				}
			}
			if agent == "claude-code" {
				backups, err := filepath.Glob(filepath.Join(home, ".claude", ".claude.json.bak-*"))
				if err != nil || len(backups) == 0 {
					t.Fatalf("Claude config was not backed up: %v, %v", backups, err)
				}
			}
			repeated, err := Run(opts)
			if err != nil {
				t.Fatal(err)
			}
			if hasStatus(repeated, "installed") || hasStatus(repeated, "registered") || hasStatus(repeated, "conflict_skip") {
				t.Fatalf("rerun changed state: %+v", repeated)
			}
			assertNoKey(t, home, key)
			opts.Uninstall = true
			removed, err := Run(opts)
			if err != nil {
				t.Fatal(err)
			}
			if !hasStatus(removed, "removed") {
				t.Fatalf("uninstall did not remove managed files: %+v", removed)
			}
			if agent != "claude-desktop" {
				if _, err := os.Stat(filepath.Join(home, agents[agent])); !os.IsNotExist(err) {
					t.Fatalf("skill remains after uninstall: %v", err)
				}
			}
			assertNoKey(t, home, key)
		})
	}
}

func TestSetupRestoresPreexistingSkillAndRejectsMissingMCPBinary(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	path := filepath.Join(home, agents["codex"])
	if err := os.MkdirAll(path, 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(path, "SKILL.md"), []byte("original user skill\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(path, "user.txt"), []byte("keep"), 0o600); err != nil {
		t.Fatal(err)
	}
	bad := Options{Agent: "codex", MCPBinary: filepath.Join(home, "missing")}
	if _, err := Run(bad); err == nil {
		t.Fatal("setup accepted missing MCP binary")
	}
	b, err := os.ReadFile(filepath.Join(path, "SKILL.md"))
	if err != nil || string(b) != "original user skill\n" {
		t.Fatalf("preflight changed user skill: %q, %v", b, err)
	}
	mcp := filepath.Join(t.TempDir(), "mixlayer-mcp")
	if err := os.WriteFile(mcp, []byte("#!/bin/sh\nexit 0\n"), 0o700); err != nil {
		t.Fatal(err)
	}
	opts := Options{Agent: "codex", MCPBinary: mcp}
	if _, err := Run(opts); err != nil {
		t.Fatal(err)
	}
	opts.Uninstall = true
	if _, err := Run(opts); err != nil {
		t.Fatal(err)
	}
	b, err = os.ReadFile(filepath.Join(path, "SKILL.md"))
	if err != nil || string(b) != "original user skill\n" {
		t.Fatalf("preexisting skill not restored: %q, %v", b, err)
	}
	if _, err := os.Stat(filepath.Join(path, "user.txt")); err != nil {
		t.Fatalf("preexisting file missing: %v", err)
	}
}

func TestSetupPreservesUnrelatedConfigAndBacksUpBeforeEdits(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	mcp := filepath.Join(t.TempDir(), "mixlayer-mcp")
	if err := os.WriteFile(mcp, []byte("#!/bin/sh\nexit 0\n"), 0o700); err != nil {
		t.Fatal(err)
	}
	cursorPath := filepath.Join(home, ".cursor", "mcp.json")
	if err := os.MkdirAll(filepath.Dir(cursorPath), 0o700); err != nil {
		t.Fatal(err)
	}
	original := []byte("{\"mcpServers\":{\"other\":{\"command\":\"other-bin\"}},\"custom\":true}\n")
	if err := os.WriteFile(cursorPath, original, 0o600); err != nil {
		t.Fatal(err)
	}
	opts := Options{Agent: "cursor", MCPBinary: mcp}
	if _, err := Run(opts); err != nil {
		t.Fatal(err)
	}
	if matches, err := filepath.Glob(cursorPath + ".bak-*"); err != nil || len(matches) == 0 {
		t.Fatalf("no config backup: %v, %v", matches, err)
	}
	opts.Uninstall = true
	if _, err := Run(opts); err != nil {
		t.Fatal(err)
	}
	b, err := os.ReadFile(cursorPath)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Contains(b, []byte(`"other"`)) || !bytes.Contains(b, []byte(`"custom": true`)) || bytes.Contains(b, []byte(`"mixlayer"`)) {
		t.Fatalf("uninstall changed unrelated config: %s", b)
	}
}

func TestCodexTOMLMergePreservesOriginalBytes(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	mcp := filepath.Join(t.TempDir(), "mixlayer-mcp")
	if err := os.WriteFile(mcp, []byte("#!/bin/sh\nexit 0\n"), 0o700); err != nil {
		t.Fatal(err)
	}
	config := filepath.Join(home, ".codex", "config.toml")
	if err := os.MkdirAll(filepath.Dir(config), 0o700); err != nil {
		t.Fatal(err)
	}
	original := []byte("# user's comment\n[other]\nmode = 'keep'\n")
	if err := os.WriteFile(config, original, 0o600); err != nil {
		t.Fatal(err)
	}
	opts := Options{Agent: "codex", MCPBinary: mcp}
	if _, err := Run(opts); err != nil {
		t.Fatal(err)
	}
	if backups, err := filepath.Glob(config + ".bak-*"); err != nil || len(backups) == 0 {
		t.Fatalf("missing TOML backup: %v, %v", backups, err)
	}
	opts.Uninstall = true
	if _, err := Run(opts); err != nil {
		t.Fatal(err)
	}
	got, err := os.ReadFile(config)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(got, original) {
		t.Fatalf("original TOML changed after uninstall: %q", got)
	}
}

func TestClaudeModifiedEntryIsNotRemoved(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("CLAUDE_CONFIG_DIR", filepath.Join(home, ".claude"))
	fakeDir := t.TempDir()
	stub := "#!/bin/sh\nconfig=\"$CLAUDE_CONFIG_DIR/.claude.json\"\nif [ \"$2\" = add ]; then printf '{\"mcpServers\":{\"mixlayer\":{\"command\":\"owned\"}}}\\n' > \"$config\"; else printf '{}\\n' > \"$config\"; fi\n"
	if err := os.WriteFile(filepath.Join(fakeDir, "claude"), []byte(stub), 0o700); err != nil {
		t.Fatal(err)
	}
	t.Setenv("PATH", fakeDir+string(os.PathListSeparator)+os.Getenv("PATH"))
	mcp := filepath.Join(fakeDir, "mixlayer-mcp")
	if err := os.WriteFile(mcp, []byte("#!/bin/sh\nexit 0\n"), 0o700); err != nil {
		t.Fatal(err)
	}
	opts := Options{Agent: "claude-code", MCPBinary: mcp}
	if _, err := Run(opts); err != nil {
		t.Fatal(err)
	}
	config := filepath.Join(home, ".claude", ".claude.json")
	changed := []byte("{\"mcpServers\":{\"mixlayer\":{\"command\":\"user-replaced\"}}}\n")
	if err := os.WriteFile(config, changed, 0o600); err != nil {
		t.Fatal(err)
	}
	opts.Uninstall = true
	report, err := Run(opts)
	if err != nil {
		t.Fatal(err)
	}
	if !hasStatus(report, "modified_skip") {
		t.Fatalf("modified MCP was not protected: %+v", report)
	}
	got, err := os.ReadFile(config)
	if err != nil || !bytes.Equal(got, changed) {
		t.Fatalf("modified MCP entry changed: %q, %v", got, err)
	}
}

func hasStatus(report Report, status string) bool {
	for _, action := range report.Actions {
		if action.Status == status {
			return true
		}
	}
	return false
}

func assertNoKey(t *testing.T, root, key string) {
	t.Helper()
	err := filepath.WalkDir(root, func(path string, d os.DirEntry, err error) error {
		if err != nil {
			return err
		}
		if d.IsDir() {
			return nil
		}
		b, err := os.ReadFile(path)
		if err != nil {
			return err
		}
		if strings.Contains(string(b), key) {
			t.Errorf("synthetic API key appeared in %s", path)
		}
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
}
