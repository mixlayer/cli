// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.
package cli

import (
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"

	"github.com/mixlayer/cli/internal/agentsetup"
	mixlayerskill "github.com/mixlayer/cli/skills/mixlayer"
	"github.com/spf13/cobra"
	"github.com/spf13/pflag"
)

func newAgentCmd(root *cobra.Command) *cobra.Command {
	cmd := &cobra.Command{Use: "agent", Short: "Install Mixlayer for coding agents and inspect the embedded skill", Annotations: map[string]string{"mcp:hidden": "true"}}
	setup := &cobra.Command{Use: "setup", Short: "Preview or install the skill and MCP server for agents", RunE: func(c *cobra.Command, _ []string) error {
		name, _ := c.Flags().GetString("agent")
		uninstall, _ := c.Flags().GetBool("uninstall")
		yes, _ := c.Flags().GetBool("yes")
		explicitYes := yes && (c.Flags().Changed("yes") || c.InheritedFlags().Changed("yes"))
		dry, _ := c.Flags().GetBool("dry-run")
		if !dry {
			dry, _ = c.InheritedFlags().GetBool("dry-run")
		}
		mcpBinary, _ := agentsetup.MCPBinary()
		report, err := agentsetup.Run(agentsetup.Options{Agent: name, MCPBinary: mcpBinary, DryRun: dry || !explicitYes, Uninstall: uninstall})
		if err != nil {
			return err
		}
		enc := json.NewEncoder(c.OutOrStdout())
		enc.SetIndent("", "  ")
		return enc.Encode(report)
	}}
	// This command intentionally shadows the root --agent boolean: here the
	// value chooses the target agent, and --yes alone authorizes local edits.
	setup.Flags().String("agent", "all", "Target: claude-code, codex, cursor, hermes, claude-desktop, or all")
	setup.Flags().Bool("uninstall", false, "Remove only integrations installed by this setup command")
	cmd.AddCommand(setup)

	skill := &cobra.Command{Use: "skill", Short: "Read or locate the embedded Agent Skill"}
	skill.AddCommand(&cobra.Command{Use: "print", Short: "Print the embedded SKILL.md", RunE: func(c *cobra.Command, _ []string) error {
		_, err := fmt.Fprint(c.OutOrStdout(), mixlayerskill.Text())
		return err
	}})
	skill.AddCommand(&cobra.Command{Use: "path", Short: "Extract the embedded skill and print its directory", RunE: func(c *cobra.Command, _ []string) error {
		cache, err := os.UserCacheDir()
		if err != nil {
			return err
		}
		hash := sha256.Sum256([]byte(mixlayerskill.Text()))
		path := filepath.Join(cache, "mixlayer", fmt.Sprintf("skill-%x", hash[:8]))
		if err := agentsetup.MaterializeSkill(path); err != nil {
			return err
		}
		_, err = fmt.Fprintln(c.OutOrStdout(), path)
		return err
	}})
	cmd.AddCommand(skill)

	// go generate renders this same Cobra tree and captures stdout in
	// the embedded reference file, so command and flag inventory cannot drift.
	ref := &cobra.Command{Use: "skill-ref", Hidden: true, RunE: func(c *cobra.Command, _ []string) error {
		_, err := fmt.Fprint(c.OutOrStdout(), generatedCommandReference(root))
		return err
	}}
	cmd.AddCommand(ref)
	return cmd
}

func generatedCommandReference(root *cobra.Command) string {
	var b strings.Builder
	b.WriteString("# Mixlayer commands\n\nGenerated from the Cobra command tree by `go generate ./skills/mixlayer`. Do not edit manually. Use `--help` for detailed syntax.\n\n")
	var visit func(*cobra.Command)
	visit = func(cmd *cobra.Command) {
		children := cmd.Commands()
		sort.Slice(children, func(i, j int) bool { return children[i].Name() < children[j].Name() })
		for _, child := range children {
			if child.Hidden || child.Name() == "completion" || child.Name() == "help" {
				continue
			}
			if child.Runnable() {
				fmt.Fprintf(&b, "## `%s`\n\n%s\n\n", child.CommandPath(), child.Short)
				var names []string
				seen := map[string]bool{}
				add := func(f *pflag.Flag) {
					if !f.Hidden && !seen[f.Name] {
						names = append(names, "--"+f.Name)
						seen[f.Name] = true
					}
				}
				child.LocalFlags().VisitAll(add)
				child.InheritedFlags().VisitAll(add)
				sort.Strings(names)
				if len(names) > 0 {
					fmt.Fprintf(&b, "Flags: %s\n\n", strings.Join(names, ", "))
				}
			}
			visit(child)
		}
	}
	visit(root)
	return strings.TrimRight(b.String(), "\n") + "\n"
}
