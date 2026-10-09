// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.
package agentsetup

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/json"
	"errors"
	"fmt"
	"io/fs"
	"os"
	"os/exec"
	"path/filepath"
	"reflect"
	"runtime"
	"strconv"
	"strings"
	"time"

	mixlayerskill "github.com/mixlayer/cli/skills/mixlayer"
	"github.com/pelletier/go-toml/v2"
)

// Skill locations must be live-verified against each agent's current release.
// Codex and Cursor skill path conventions are especially subject to change.
var agents = map[string]string{
	"claude-code": ".claude/skills/mixlayer",
	"codex":       ".codex/skills/mixlayer",
	"cursor":      ".cursor/skills/mixlayer",
	"hermes":      ".hermes/skills/mixlayer",
}

var order = []string{"claude-code", "codex", "cursor", "hermes", "claude-desktop"}

type Options struct {
	Agent, MCPBinary  string
	DryRun, Uninstall bool
}

type Action struct {
	Agent   string `json:"agent"`
	Kind    string `json:"kind"`
	Path    string `json:"path,omitempty"`
	Status  string `json:"status"`
	Command string `json:"command,omitempty"`
	Note    string `json:"note,omitempty"`
}

type Report struct {
	DryRun    bool     `json:"dry_run"`
	Uninstall bool     `json:"uninstall"`
	Actions   []Action `json:"actions"`
}

type entry struct {
	SkillInstalled bool            `json:"skill_installed,omitempty"`
	SkillBackup    string          `json:"skill_backup,omitempty"`
	MCPInstalled   bool            `json:"mcp_installed,omitempty"`
	MCPFileCreated bool            `json:"mcp_file_created,omitempty"`
	MCPValue       json.RawMessage `json:"mcp_value,omitempty"`
	MCPHash        string          `json:"mcp_hash,omitempty"`
	MCPSnippet     string          `json:"mcp_snippet,omitempty"`
}

type state map[string]entry

func Targets(name string) ([]string, error) {
	if name == "" || name == "all" {
		return append([]string(nil), order...), nil
	}
	for _, candidate := range order {
		if name == candidate {
			return []string{name}, nil
		}
	}
	return nil, fmt.Errorf("unknown agent %q (choose claude-code, codex, cursor, hermes, claude-desktop, or all)", name)
}

func MCPBinary() (string, error) {
	if candidate := os.Getenv("MIXLAYER_MCP_PATH"); candidate != "" {
		return filepath.Abs(candidate)
	}
	name := "mixlayer-mcp"
	if runtime.GOOS == "windows" {
		name += ".exe"
	}
	if exe, err := os.Executable(); err == nil {
		candidate := filepath.Join(filepath.Dir(exe), name)
		if info, err := os.Stat(candidate); err == nil && !info.IsDir() {
			return candidate, nil
		}
	}
	return exec.LookPath(name)
}

func Run(opts Options) (Report, error) {
	targets, err := Targets(opts.Agent)
	if err != nil {
		return Report{}, err
	}
	if !opts.DryRun && !opts.Uninstall {
		needsMCP := false
		for _, agent := range targets {
			if agent != "hermes" {
				needsMCP = true
			}
		}
		if needsMCP {
			bin := opts.MCPBinary
			if bin == "" {
				bin, _ = MCPBinary()
			}
			info, err := os.Stat(bin)
			if err != nil || info.IsDir() || info.Mode().Perm()&0o111 == 0 {
				return Report{}, fmt.Errorf("mixlayer-mcp executable not found; build or install it beside mixlayer before setup")
			}
			opts.MCPBinary = bin
		}
	}
	home, err := os.UserHomeDir()
	if err != nil {
		return Report{}, err
	}
	report := Report{DryRun: opts.DryRun, Uninstall: opts.Uninstall, Actions: []Action{}}
	statePath := filepath.Join(home, ".config", "mixlayer", "agent-setup.json")
	st, err := loadState(statePath)
	if err != nil {
		return report, err
	}
	stateChanged := false
	for _, agent := range targets {
		rec := st[agent]
		prior := rec
		if rel, ok := agents[agent]; ok {
			path := filepath.Join(home, rel)
			changed, err := skillAction(&report, agent, path, &rec, opts)
			if err != nil {
				if stateChanged && !opts.DryRun {
					if saveErr := writeState(statePath, st); saveErr != nil {
						return report, fmt.Errorf("%w; saving partial setup state: %w", err, saveErr)
					}
				}
				return report, err
			}
			stateChanged = stateChanged || changed
			if changed {
				st[agent] = rec
			}
		}
		changed, err := mcpAction(&report, agent, home, &rec, opts)
		if err != nil {
			if !opts.DryRun && (stateChanged || !reflect.DeepEqual(rec, prior)) {
				if reflect.DeepEqual(rec, entry{}) {
					delete(st, agent)
				} else {
					st[agent] = rec
				}
				if saveErr := writeState(statePath, st); saveErr != nil {
					return report, fmt.Errorf("%w; saving partial setup state: %w", err, saveErr)
				}
			}
			return report, err
		}
		stateChanged = stateChanged || changed
		if !opts.DryRun {
			if reflect.DeepEqual(rec, entry{}) {
				delete(st, agent)
			} else {
				st[agent] = rec
			}
		}
	}
	if stateChanged && !opts.DryRun {
		if len(st) == 0 {
			if _, err := os.Stat(statePath); err == nil {
				if err := backupFile(statePath); err != nil {
					return report, err
				}
				if err := os.Remove(statePath); err != nil {
					return report, err
				}
			}
		} else if err := writeState(statePath, st); err != nil {
			return report, err
		}
	}
	return report, nil
}

func loadState(path string) (state, error) {
	b, err := os.ReadFile(path)
	if errors.Is(err, os.ErrNotExist) {
		return state{}, nil
	}
	if err != nil {
		return nil, err
	}
	var st state
	if err := json.Unmarshal(b, &st); err != nil {
		return nil, fmt.Errorf("reading setup state: %w", err)
	}
	if st == nil {
		st = state{}
	}
	return st, nil
}

func writeState(path string, st state) error {
	b, err := json.MarshalIndent(st, "", "  ")
	if err != nil {
		return err
	}
	b = append(b, '\n')
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		return err
	}
	if old, err := os.ReadFile(path); err == nil {
		if bytes.Equal(old, b) {
			return nil
		}
		if err := backupFile(path); err != nil {
			return err
		}
	} else if !errors.Is(err, os.ErrNotExist) {
		return err
	}
	return os.WriteFile(path, b, 0o600)
}

func backupFile(path string) error {
	b, err := os.ReadFile(path)
	if err != nil {
		return err
	}
	info, err := os.Stat(path)
	if err != nil {
		return err
	}
	backup := path + ".bak-" + time.Now().UTC().Format("20060102T150405.000000000Z")
	return os.WriteFile(backup, b, info.Mode().Perm())
}

func skillAction(report *Report, agent, path string, rec *entry, opts Options) (bool, error) {
	action := Action{Agent: agent, Kind: "skill", Path: path}
	_, statErr := os.Stat(path)
	exists := statErr == nil
	if statErr != nil && !errors.Is(statErr, os.ErrNotExist) {
		return false, statErr
	}
	if opts.Uninstall {
		if !rec.SkillInstalled {
			action.Status = "not_managed"
			report.Actions = append(report.Actions, action)
			return false, nil
		}
		if !exists {
			if opts.DryRun {
				action.Status = "would_restore"
			} else {
				if rec.SkillBackup != "" {
					if err := os.Rename(rec.SkillBackup, path); err != nil {
						return false, err
					}
				}
				rec.SkillInstalled = false
				rec.SkillBackup = ""
				action.Status = "removed"
			}
			report.Actions = append(report.Actions, action)
			return !opts.DryRun, nil
		}
		match, err := skillMatches(path)
		if err != nil {
			return false, err
		}
		if !match {
			action.Status = "modified_skip"
			report.Actions = append(report.Actions, action)
			return false, nil
		}
		if opts.DryRun {
			action.Status = "would_remove"
		} else {
			if err := os.RemoveAll(path); err != nil {
				return false, err
			}
			if rec.SkillBackup != "" {
				if err := os.Rename(rec.SkillBackup, path); err != nil {
					return false, err
				}
			}
			rec.SkillInstalled = false
			rec.SkillBackup = ""
			action.Status = "removed"
		}
		report.Actions = append(report.Actions, action)
		return !opts.DryRun, nil
	}
	if exists {
		match, err := skillMatches(path)
		if err != nil {
			return false, err
		}
		if match {
			action.Status = "unchanged"
			report.Actions = append(report.Actions, action)
			return false, nil
		}
		if rec.SkillInstalled {
			action.Status = "modified_skip"
			report.Actions = append(report.Actions, action)
			return false, nil
		}
	}
	if opts.DryRun {
		action.Status = "would_install"
		if exists {
			action.Note = "existing skill will be backed up"
		}
		report.Actions = append(report.Actions, action)
		return false, nil
	}
	if exists {
		backup := path + ".bak-" + time.Now().UTC().Format("20060102T150405.000000000Z")
		if err := os.Rename(path, backup); err != nil {
			return false, err
		}
		rec.SkillBackup = backup
	}
	if err := installSkill(path); err != nil {
		_ = os.RemoveAll(path)
		if rec.SkillBackup != "" {
			_ = os.Rename(rec.SkillBackup, path)
			rec.SkillBackup = ""
		}
		return false, err
	}
	rec.SkillInstalled = true
	action.Status = "installed"
	report.Actions = append(report.Actions, action)
	return true, nil
}

func installSkill(path string) error {
	return fs.WalkDir(mixlayerskill.Files, ".", func(name string, d fs.DirEntry, err error) error {
		if err != nil {
			return err
		}
		if d.IsDir() {
			return os.MkdirAll(filepath.Join(path, name), 0o700)
		}
		b, err := mixlayerskill.Files.ReadFile(name)
		if err != nil {
			return err
		}
		return os.WriteFile(filepath.Join(path, name), b, 0o600)
	})
}

func skillMatches(path string) (bool, error) {
	match := true
	expected := map[string]bool{}
	err := fs.WalkDir(mixlayerskill.Files, ".", func(name string, d fs.DirEntry, err error) error {
		if err != nil {
			return err
		}
		if d.IsDir() {
			return nil
		}
		expected[filepath.Clean(name)] = true
		want, err := mixlayerskill.Files.ReadFile(name)
		if err != nil {
			return err
		}
		info, err := os.Lstat(filepath.Join(path, name))
		if errors.Is(err, os.ErrNotExist) {
			match = false
			return nil
		}
		if err != nil {
			return err
		}
		if !info.Mode().IsRegular() {
			match = false
			return nil
		}
		got, err := os.ReadFile(filepath.Join(path, name))
		if errors.Is(err, os.ErrNotExist) {
			match = false
			return nil
		}
		if err != nil {
			return err
		}
		if !bytes.Equal(got, want) {
			match = false
		}
		return nil
	})
	if err != nil {
		return false, err
	}
	err = filepath.WalkDir(path, func(name string, d fs.DirEntry, err error) error {
		if err != nil {
			return err
		}
		if d.IsDir() {
			return nil
		}
		rel, err := filepath.Rel(path, name)
		if err != nil {
			return err
		}
		if !expected[filepath.Clean(rel)] {
			match = false
		}
		return nil
	})
	return match, err
}

func mcpAction(report *Report, agent, home string, rec *entry, opts Options) (bool, error) {
	if agent == "hermes" {
		report.Actions = append(report.Actions, Action{Agent: agent, Kind: "mcp", Status: "unsupported", Note: "skill only; MCP config path is not established"})
		return false, nil
	}
	bin := opts.MCPBinary
	if bin == "" {
		bin, _ = MCPBinary()
	}
	if !opts.Uninstall && bin == "" {
		report.Actions = append(report.Actions, Action{Agent: agent, Kind: "mcp", Status: "missing_binary", Note: "build or install mixlayer-mcp beside mixlayer"})
		return false, nil
	}
	if !opts.Uninstall {
		info, err := os.Stat(bin)
		if err != nil || info.IsDir() || info.Mode().Perm()&0o111 == 0 {
			report.Actions = append(report.Actions, Action{Agent: agent, Kind: "mcp", Status: "missing_binary", Path: bin, Note: "mixlayer-mcp must be an executable file"})
			return false, nil
		}
	}
	switch agent {
	case "claude-code":
		return claudeMCP(report, home, bin, rec, opts)
	case "cursor":
		return jsonMCP(report, agent, filepath.Join(home, ".cursor", "mcp.json"), bin, rec, opts)
	case "claude-desktop":
		base := filepath.Join(home, "Library", "Application Support", "Claude")
		if runtime.GOOS == "windows" && os.Getenv("APPDATA") != "" {
			base = filepath.Join(os.Getenv("APPDATA"), "Claude")
		}
		if runtime.GOOS == "linux" {
			base = filepath.Join(home, ".config", "Claude")
		}
		return jsonMCP(report, agent, filepath.Join(base, "claude_desktop_config.json"), bin, rec, opts)
	case "codex":
		return tomlMCP(report, agent, filepath.Join(home, ".codex", "config.toml"), bin, rec, opts)
	}
	return false, nil
}

func jsonMCP(report *Report, agent, path, bin string, rec *entry, opts Options) (bool, error) {
	action := Action{Agent: agent, Kind: "mcp", Path: path}
	b, err := os.ReadFile(path)
	created := errors.Is(err, os.ErrNotExist)
	if created {
		b = []byte("{}")
	} else if err != nil {
		return false, err
	}
	var doc map[string]any
	if err := json.Unmarshal(b, &doc); err != nil {
		return false, fmt.Errorf("%s: %w", path, err)
	}
	if doc == nil {
		doc = map[string]any{}
	}
	servers, ok := doc["mcpServers"].(map[string]any)
	if !ok && doc["mcpServers"] != nil {
		return false, fmt.Errorf("%s: mcpServers must be an object", path)
	}
	if servers == nil {
		servers = map[string]any{}
	}
	current, exists := servers["mixlayer"]
	if opts.Uninstall {
		if !rec.MCPInstalled {
			action.Status = "not_managed"
			report.Actions = append(report.Actions, action)
			return false, nil
		}
		var owned any
		if err := json.Unmarshal(rec.MCPValue, &owned); err != nil {
			return false, err
		}
		if !exists || !reflect.DeepEqual(current, owned) {
			action.Status = "modified_skip"
			report.Actions = append(report.Actions, action)
			return false, nil
		}
		if opts.DryRun {
			action.Status = "would_remove"
			report.Actions = append(report.Actions, action)
			return false, nil
		}
		delete(servers, "mixlayer")
		if len(servers) == 0 {
			delete(doc, "mcpServers")
		} else {
			doc["mcpServers"] = servers
		}
		if rec.MCPFileCreated && len(doc) == 0 {
			if err := backupFile(path); err != nil {
				return false, err
			}
			if err := os.Remove(path); err != nil {
				return false, err
			}
		} else if err := writeJSONConfig(path, doc); err != nil {
			return false, err
		}
		rec.MCPInstalled = false
		rec.MCPFileCreated = false
		rec.MCPValue = nil
		action.Status = "removed"
		report.Actions = append(report.Actions, action)
		return true, nil
	}
	value := map[string]any{"command": bin, "args": []string{}}
	if exists {
		if equalJSONValues(current, value) {
			action.Status = "unchanged"
		} else {
			action.Status = "conflict_skip"
		}
		report.Actions = append(report.Actions, action)
		return false, nil
	}
	if opts.DryRun {
		action.Status = "would_register"
		report.Actions = append(report.Actions, action)
		return false, nil
	}
	servers["mixlayer"] = value
	doc["mcpServers"] = servers
	if err := writeJSONConfig(path, doc); err != nil {
		return false, err
	}
	rec.MCPInstalled = true
	rec.MCPFileCreated = created
	rec.MCPValue, _ = json.Marshal(value)
	action.Status = "registered"
	report.Actions = append(report.Actions, action)
	return true, nil
}

func writeJSONConfig(path string, doc map[string]any) error {
	b, err := json.MarshalIndent(doc, "", "  ")
	if err != nil {
		return err
	}
	b = append(b, '\n')
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		return err
	}
	if _, err := os.Stat(path); err == nil {
		if err := backupFile(path); err != nil {
			return err
		}
	} else if !errors.Is(err, os.ErrNotExist) {
		return err
	}
	return os.WriteFile(path, b, 0o600)
}

func tomlMCP(report *Report, agent, path, bin string, rec *entry, opts Options) (bool, error) {
	action := Action{Agent: agent, Kind: "mcp", Path: path}
	b, err := os.ReadFile(path)
	created := errors.Is(err, os.ErrNotExist)
	if created {
		b = nil
	} else if err != nil {
		return false, err
	}
	var doc map[string]any
	if len(b) > 0 {
		if err := toml.Unmarshal(b, &doc); err != nil {
			return false, fmt.Errorf("%s: %w", path, err)
		}
	}
	if doc == nil {
		doc = map[string]any{}
	}
	servers, ok := doc["mcp_servers"].(map[string]any)
	if !ok && doc["mcp_servers"] != nil {
		return false, fmt.Errorf("%s: mcp_servers must be a table", path)
	}
	_, exists := servers["mixlayer"]
	if opts.Uninstall {
		if !rec.MCPInstalled {
			action.Status = "not_managed"
			report.Actions = append(report.Actions, action)
			return false, nil
		}
		if rec.MCPSnippet == "" || !strings.Contains(string(b), rec.MCPSnippet) {
			action.Status = "modified_skip"
			report.Actions = append(report.Actions, action)
			return false, nil
		}
		if opts.DryRun {
			action.Status = "would_remove"
			report.Actions = append(report.Actions, action)
			return false, nil
		}
		updated := strings.Replace(string(b), rec.MCPSnippet, "", 1)
		if rec.MCPFileCreated && strings.TrimSpace(updated) == "" {
			if err := backupFile(path); err != nil {
				return false, err
			}
			if err := os.Remove(path); err != nil {
				return false, err
			}
		} else if err := writeTextConfig(path, []byte(updated)); err != nil {
			return false, err
		}
		rec.MCPInstalled = false
		rec.MCPFileCreated = false
		rec.MCPSnippet = ""
		action.Status = "removed"
		report.Actions = append(report.Actions, action)
		return true, nil
	}
	if exists {
		if rec.MCPInstalled {
			action.Status = "unchanged"
		} else {
			action.Status = "conflict_skip"
		}
		report.Actions = append(report.Actions, action)
		return false, nil
	}
	if opts.DryRun {
		action.Status = "would_register"
		report.Actions = append(report.Actions, action)
		return false, nil
	}
	prefix := "\n"
	if len(b) == 0 {
		prefix = ""
	} else if b[len(b)-1] != '\n' {
		prefix = "\n\n"
	}
	snippet := prefix + "# mixlayer agent setup begin\n[mcp_servers.mixlayer]\ncommand = " + strconv.Quote(bin) + "\nargs = []\n# mixlayer agent setup end\n"
	if err := writeTextConfig(path, append(b, snippet...)); err != nil {
		return false, err
	}
	rec.MCPInstalled = true
	rec.MCPFileCreated = created
	rec.MCPSnippet = snippet
	action.Status = "registered"
	report.Actions = append(report.Actions, action)
	return true, nil
}

func writeTextConfig(path string, b []byte) error {
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		return err
	}
	if _, err := os.Stat(path); err == nil {
		if err := backupFile(path); err != nil {
			return err
		}
	} else if !errors.Is(err, os.ErrNotExist) {
		return err
	}
	return os.WriteFile(path, b, 0o600)
}

func claudeMCP(report *Report, home, bin string, rec *entry, opts Options) (bool, error) {
	configPath := filepath.Join(home, ".claude.json")
	if configDir := os.Getenv("CLAUDE_CONFIG_DIR"); configDir != "" {
		configPath = filepath.Join(configDir, ".claude.json")
	}
	action := Action{Agent: "claude-code", Kind: "mcp", Path: configPath}
	claude, err := exec.LookPath("claude")
	if err != nil {
		action.Status = "manual_command"
		if opts.Uninstall {
			action.Command = "claude mcp remove --scope user mixlayer"
		} else {
			action.Command = "claude mcp add --scope user mixlayer -- " + strconv.Quote(bin)
		}
		report.Actions = append(report.Actions, action)
		return false, nil
	}
	if opts.Uninstall && !rec.MCPInstalled {
		action.Status = "not_managed"
		report.Actions = append(report.Actions, action)
		return false, nil
	}
	current, exists, err := readClaudeMCPEntry(action.Path)
	if err != nil {
		return false, err
	}
	if !opts.Uninstall && rec.MCPInstalled {
		if exists && hashJSON(current) == rec.MCPHash {
			action.Status = "unchanged"
		} else {
			action.Status = "modified_skip"
		}
		report.Actions = append(report.Actions, action)
		return false, nil
	}
	if !opts.Uninstall && exists {
		action.Status = "conflict_skip"
		report.Actions = append(report.Actions, action)
		return false, nil
	}
	if opts.Uninstall {
		if !exists {
			if opts.DryRun {
				action.Status = "would_remove"
			} else {
				rec.MCPInstalled = false
				rec.MCPHash = ""
				action.Status = "removed"
			}
			report.Actions = append(report.Actions, action)
			return !opts.DryRun, nil
		}
		if hashJSON(current) != rec.MCPHash {
			action.Status = "modified_skip"
			report.Actions = append(report.Actions, action)
			return false, nil
		}
	}
	args := []string{"mcp", "add", "--scope", "user", "mixlayer", "--", bin}
	if opts.Uninstall {
		args = []string{"mcp", "remove", "--scope", "user", "mixlayer"}
	}
	action.Command = "claude " + strings.Join(args, " ")
	if opts.DryRun {
		if opts.Uninstall {
			action.Status = "would_remove"
		} else {
			action.Status = "would_register"
		}
		report.Actions = append(report.Actions, action)
		return false, nil
	}
	if _, err := os.Stat(action.Path); err == nil {
		if err := backupFile(action.Path); err != nil {
			return false, err
		}
	} else if !errors.Is(err, os.ErrNotExist) {
		return false, err
	}
	cmd := exec.CommandContext(context.Background(), claude, args...)
	cmd.Env = os.Environ()
	out, err := cmd.CombinedOutput()
	if err != nil {
		return false, fmt.Errorf("claude mcp command failed: %w: %s", err, strings.TrimSpace(string(out)))
	}
	if opts.Uninstall {
		rec.MCPInstalled = false
		rec.MCPHash = ""
		action.Status = "removed"
	} else {
		value, exists, err := readClaudeMCPEntry(action.Path)
		if err != nil {
			return false, err
		}
		if !exists {
			return false, fmt.Errorf("claude mcp add succeeded but %s has no mixlayer entry", action.Path)
		}
		rec.MCPInstalled = true
		rec.MCPHash = hashJSON(value)
		action.Status = "registered"
	}
	report.Actions = append(report.Actions, action)
	return true, nil
}

// Decode only the MCP entry needed for ownership checks. Other Claude config
// values are never included in reports or setup state.
func readClaudeMCPEntry(path string) (json.RawMessage, bool, error) {
	b, err := os.ReadFile(path)
	if errors.Is(err, os.ErrNotExist) {
		return nil, false, nil
	}
	if err != nil {
		return nil, false, err
	}
	var doc struct {
		MCPServers map[string]json.RawMessage `json:"mcpServers"`
	}
	if err := json.Unmarshal(b, &doc); err != nil {
		return nil, false, fmt.Errorf("reading Claude MCP metadata: %w", err)
	}
	value, ok := doc.MCPServers["mixlayer"]
	return value, ok, nil
}

func hashJSON(data json.RawMessage) string {
	var v any
	if json.Unmarshal(data, &v) != nil {
		return ""
	}
	canonical, err := json.Marshal(v)
	if err != nil {
		return ""
	}
	hash := sha256.Sum256(canonical)
	return fmt.Sprintf("%x", hash[:])
}

func equalJSONValues(a, b any) bool {
	left, err := json.Marshal(a)
	if err != nil {
		return false
	}
	right, err := json.Marshal(b)
	if err != nil {
		return false
	}
	return hashJSON(left) == hashJSON(right)
}

// MaterializeSkill writes the embedded files beneath a caller-provided path.
func MaterializeSkill(path string) error {
	if _, err := os.Stat(path); err == nil {
		match, err := skillMatches(path)
		if err != nil {
			return err
		}
		if !match {
			return fmt.Errorf("embedded skill cache at %s was modified", path)
		}
		return nil
	} else if !errors.Is(err, os.ErrNotExist) {
		return err
	}
	return installSkill(path)
}
