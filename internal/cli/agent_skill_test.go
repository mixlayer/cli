// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.
package cli

import (
	"bufio"
	"bytes"
	"encoding/json"
	"os"
	"path/filepath"
	"regexp"
	"strings"
	"testing"

	"github.com/spf13/cobra"
)

var inlineCommand = regexp.MustCompile("`(mixlayer [^`]+)`")

// TestSkillCommandLinesMatchCobraTree is the drift guard for every documented
// executable example, including the generated reference.
func TestSkillCommandLinesMatchCobraTree(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	root := RootCmd()
	paths := []string{
		"../../README.md",
		"../../skills/mixlayer/SKILL.md",
		"../../skills/mixlayer/references/privacy.md",
		"../../skills/mixlayer/references/commands.md",
	}
	count := 0
	for _, path := range paths {
		file, err := os.Open(path)
		if err != nil {
			t.Fatal(err)
		}
		scanner := bufio.NewScanner(file)
		lineNo := 0
		var heading *cobra.Command
		for scanner.Scan() {
			lineNo++
			line := strings.TrimSpace(scanner.Text())
			var commands []string
			if strings.HasPrefix(line, "mixlayer ") {
				commands = append(commands, line)
			}
			for _, match := range inlineCommand.FindAllStringSubmatch(line, -1) {
				commands = append(commands, match[1])
			}
			for _, text := range commands {
				cmd := validateSkillCommand(t, root, path, lineNo, text)
				count++
				if strings.HasPrefix(line, "## `mixlayer ") {
					heading = cmd
				}
			}
			if strings.HasPrefix(line, "Flags: ") && heading != nil {
				for _, part := range strings.Split(strings.TrimPrefix(line, "Flags: "), ",") {
					flag := strings.TrimSpace(strings.TrimPrefix(strings.TrimSpace(part), "--"))
					if !skillFlagExists(root, heading, flag) {
						t.Errorf("%s:%d: generated flag --%s absent from %s", path, lineNo, flag, heading.CommandPath())
					}
				}
			}
		}
		if err := scanner.Err(); err != nil {
			t.Fatal(err)
		}
		if err := file.Close(); err != nil {
			t.Fatal(err)
		}
	}
	if count < 60 {
		t.Fatalf("validated only %d skill commands; expected generated reference and examples", count)
	}
}

func validateSkillCommand(t *testing.T, root *cobra.Command, path string, line int, text string) *cobra.Command {
	t.Helper()
	words := strings.Fields(text)
	cmd := root
	for _, word := range words[1:] {
		if strings.HasPrefix(word, "-") {
			break
		}
		var child *cobra.Command
		for _, candidate := range cmd.Commands() {
			if candidate.Name() == word {
				child = candidate
				break
			}
		}
		if child == nil {
			break
		}
		cmd = child
	}
	if cmd == root || !cmd.Runnable() {
		t.Errorf("%s:%d: %q is not a runnable command", path, line, text)
	}
	for _, word := range words[1:] {
		if !strings.HasPrefix(word, "--") || word == "--" {
			continue
		}
		name := strings.SplitN(strings.TrimPrefix(word, "--"), "=", 2)[0]
		if !skillFlagExists(root, cmd, name) {
			t.Errorf("%s:%d: --%s absent from %s", path, line, name, cmd.CommandPath())
		}
	}
	return cmd
}

func skillFlagExists(root, cmd *cobra.Command, name string) bool {
	return cmd.LocalFlags().Lookup(name) != nil || cmd.InheritedFlags().Lookup(name) != nil || root.PersistentFlags().Lookup(name) != nil
}

func TestGeneratedCommandReferenceMatchesCobraTree(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	path := filepath.Join("..", "..", "skills", "mixlayer", "references", "commands.md")
	got, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if want := generatedCommandReference(RootCmd()); string(got) != want {
		t.Fatal("generated command reference is stale; run go generate ./skills/mixlayer")
	}
}

func TestEmbeddedSkillFrontmatterAndLength(t *testing.T) {
	b, err := os.ReadFile(filepath.Join("..", "..", "skills", "mixlayer", "SKILL.md"))
	if err != nil {
		t.Fatal(err)
	}
	text := string(b)
	if !strings.HasPrefix(text, "---\nname: mixlayer\ndescription: ") {
		t.Fatal("skill frontmatter is missing name or description")
	}
	if lines := strings.Count(text, "\n") + 1; lines >= 150 {
		t.Fatalf("skill has %d lines, want fewer than 150", lines)
	}
	for _, trigger := range []string{"OpenAI-compatible", "Claude", "GPT", "prices", "redaction", "credits"} {
		if !strings.Contains(strings.SplitN(text, "---", 3)[1], trigger) {
			t.Errorf("frontmatter description missing %q", trigger)
		}
	}
}

func TestAgentSetupRequiresTrueExplicitYes(t *testing.T) {
	for _, args := range [][]string{
		{"agent", "setup", "--agent", "codex"},
		{"agent", "setup", "--agent", "codex", "--yes=false"},
	} {
		t.Run(strings.Join(args, " "), func(t *testing.T) {
			home := t.TempDir()
			t.Setenv("HOME", home)
			root := RootCmd()
			var out bytes.Buffer
			root.SetOut(&out)
			root.SetArgs(args)
			if err := root.Execute(); err != nil {
				t.Fatal(err)
			}
			var report struct {
				DryRun bool `json:"dry_run"`
			}
			if err := json.Unmarshal(out.Bytes(), &report); err != nil {
				t.Fatal(err)
			}
			if !report.DryRun {
				t.Fatal("setup wrote without true explicit --yes")
			}
			if files, err := os.ReadDir(home); err != nil || len(files) != 0 {
				t.Fatalf("preview wrote files: %v, %v", files, err)
			}
		})
	}
}
