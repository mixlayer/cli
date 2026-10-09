// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

package cli

import (
	"encoding/json"
	"os"
	"sort"

	"github.com/mixlayer/cli/internal/cliutil"
	"github.com/spf13/cobra"
	"github.com/spf13/pflag"
)

// agentContextSchemaVersion is bumped on any breaking change to the JSON
// shape emitted by `agent-context`. Agents should check this before
// parsing. Shape at v4 adds resolved config/data/state/cache directories.
const agentContextSchemaVersion = "4"

// agentContext is the structured description of this CLI consumed by AI
// agents. Inspired by Cloudflare's /cdn-cgi/explorer/api runtime endpoint
// (2026-04-13 Wrangler post): agents can introspect the live CLI without
// parsing --help or reading source.
type agentContext struct {
	SchemaVersion     string                 `json:"schema_version"`
	CLI               agentContextCLI        `json:"cli"`
	Auth              agentContextAuth       `json:"auth"`
	ExitCodes         []agentContextExitCode `json:"exit_codes"`
	Paths             agentContextPaths      `json:"paths"`
	Discovery         *agentContextDiscovery `json:"discovery,omitempty"`
	Commands          []agentContextCommand  `json:"commands"`
	AvailableProfiles []string               `json:"available_profiles"`
}

type agentContextCLI struct {
	Name        string `json:"name"`
	Description string `json:"description"`
	Version     string `json:"version"`
}

type agentContextAuth struct {
	Mode    string                   `json:"mode"`
	EnvVars []agentContextAuthEnvVar `json:"env_vars"`
}

type agentContextAuthEnvVar struct {
	Name        string `json:"name"`
	Kind        string `json:"kind"`
	Required    bool   `json:"required"`
	Sensitive   bool   `json:"sensitive"`
	Description string `json:"description,omitempty"`
}

type agentContextExitCode struct {
	Code    int    `json:"code"`
	Meaning string `json:"meaning"`
}

type agentContextPaths struct {
	ConfigDir string `json:"config_dir"`
	DataDir   string `json:"data_dir"`
	StateDir  string `json:"state_dir"`
	CacheDir  string `json:"cache_dir"`
}

type agentContextDiscovery struct {
	Source            string   `json:"source"`
	TargetURL         string   `json:"target_url,omitempty"`
	EntryCount        int      `json:"entry_count,omitempty"`
	APIEntryCount     int      `json:"api_entry_count,omitempty"`
	Reachability      string   `json:"reachability,omitempty"`
	Protocols         []string `json:"protocols,omitempty"`
	AuthCandidates    []string `json:"auth_candidates,omitempty"`
	Protections       []string `json:"protections,omitempty"`
	GenerationHints   []string `json:"generation_hints,omitempty"`
	Warnings          []string `json:"warnings,omitempty"`
	CandidateCommands []string `json:"candidate_commands,omitempty"`
}

type agentContextCommand struct {
	Name        string                `json:"name"`
	Use         string                `json:"use,omitempty"`
	Short       string                `json:"short,omitempty"`
	Annotations map[string]string     `json:"annotations,omitempty"`
	Flags       []agentContextFlag    `json:"flags,omitempty"`
	Subcommands []agentContextCommand `json:"subcommands,omitempty"`
}

type agentContextFlag struct {
	Name    string `json:"name"`
	Type    string `json:"type"`
	Usage   string `json:"usage,omitempty"`
	Default string `json:"default,omitempty"`
}

func newAgentContextCmd(rootCmd *cobra.Command) *cobra.Command {
	var pretty bool
	cmd := &cobra.Command{
		Use:         "agent-context",
		Short:       "Emit structured JSON describing this CLI for agents",
		Example:     "  mixlayer agent-context --pretty\n  mixlayer agent-context --agent",
		Annotations: map[string]string{"mcp:read-only": "true"},
		Long: `Outputs a machine-readable description of commands, flags, and auth so
agents can introspect this CLI at runtime without parsing --help or
reading source. Schema is versioned via schema_version.`,
		RunE: func(cmd *cobra.Command, args []string) error {
			ctx := buildAgentContext(rootCmd)
			enc := json.NewEncoder(os.Stdout)
			if pretty {
				enc.SetIndent("", "  ")
			}
			return enc.Encode(ctx)
		},
	}
	cmd.Flags().BoolVar(&pretty, "pretty", false, "indent JSON output for human reading")
	return cmd
}

func buildAgentContext(rootCmd *cobra.Command) agentContext {
	envVars := []agentContextAuthEnvVar{
		{
			Name:        "MIXLAYER_API_KEY",
			Kind:        "per_call",
			Required:    true,
			Sensitive:   true,
			Description: "Set to your API credential.",
		},
	}
	authMode := "bearer_token"
	if authMode == "" {
		authMode = "none"
	}
	profiles := ListProfileNames()
	if profiles == nil {
		profiles = []string{}
	}
	return agentContext{
		SchemaVersion: agentContextSchemaVersion,
		CLI: agentContextCLI{
			Name:        "mixlayer",
			Description: "CLI for mixlayer",
			Version:     rootCmd.Version,
		},
		Auth: agentContextAuth{
			Mode:    authMode,
			EnvVars: envVars,
		},
		ExitCodes: []agentContextExitCode{
			{0, "success"},
			{1, "other failure"},
			{2, "usage error"},
			{3, "not found (HTTP 404)"},
			{4, "authentication or permission (HTTP 401/403, some HTTP 400)"},
			{5, "API failure (including other HTTP errors)"},
			{6, "partial failure in a successful response"},
			{7, "rate limited (HTTP 429; honor Retry-After)"},
			{8, "billing or insufficient prepaid credits (HTTP 402; do not retry)"},
			{9, "decision gate matched (decide --fail-if; response printed)"},
			{10, "configuration failure"},
		},
		Paths:             buildAgentContextPaths(),
		Discovery:         buildAgentDiscoveryContext(),
		Commands:          collectAgentCommands(rootCmd),
		AvailableProfiles: profiles,
	}
}

func buildAgentContextPaths() agentContextPaths {
	configDir, _ := cliutil.ConfigDir()
	dataDir, _ := cliutil.DataDir()
	stateDir, _ := cliutil.StateDir()
	cacheDir, _ := cliutil.CacheDir()
	return agentContextPaths{
		ConfigDir: configDir,
		DataDir:   dataDir,
		StateDir:  stateDir,
		CacheDir:  cacheDir,
	}
}

func buildAgentDiscoveryContext() *agentContextDiscovery {
	return nil
}

// collectAgentCommands walks the cobra tree from the given command and
// returns its direct children (skipping the agent-context command itself
// to avoid self-reference). Each child is recursed into if it has
// subcommands. Flags are captured via VisitAll. Output is sorted by
// command name for stable diffs across regenerations.
//
// Cobra's Hidden flag suppresses listing in --help but does not gate
// agent discovery. Raw resource parents are Hidden so --help stays
// curated and the `api` browser populates; the agent-context surface
// must still enumerate them and their endpoints so agents can call any
// action a CLI user could.
func collectAgentCommands(c *cobra.Command) []agentContextCommand {
	children := c.Commands()
	sort.Slice(children, func(i, j int) bool { return children[i].Name() < children[j].Name() })

	out := make([]agentContextCommand, 0, len(children))
	for _, sub := range children {
		if sub.Name() == "agent-context" || sub.Annotations[experimentalAnnotation] == "true" {
			continue
		}
		entry := agentContextCommand{
			Name:  sub.Name(),
			Use:   sub.Use,
			Short: sub.Short,
		}
		// Surface Cobra annotations (e.g., cli:endpoint, mcp:read-only) so
		// agents and test harnesses can detect destructive-at-auth
		// endpoints without parsing source. Empty maps are stripped via
		// omitempty in the struct tag.
		if len(sub.Annotations) > 0 {
			entry.Annotations = make(map[string]string, len(sub.Annotations))
			for k, v := range sub.Annotations {
				entry.Annotations[k] = v
			}
		}
		sub.Flags().VisitAll(func(f *pflag.Flag) {
			entry.Flags = append(entry.Flags, agentContextFlag{
				Name:    f.Name,
				Type:    f.Value.Type(),
				Usage:   f.Usage,
				Default: f.DefValue,
			})
		})
		sort.Slice(entry.Flags, func(i, j int) bool {
			return entry.Flags[i].Name < entry.Flags[j].Name
		})
		if len(sub.Commands()) > 0 {
			entry.Subcommands = collectAgentCommands(sub)
		}
		out = append(out, entry)
	}
	return out
}
