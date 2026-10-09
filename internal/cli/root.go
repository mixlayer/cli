// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

package cli

import (
	"fmt"
	"os"
	"strings"
	"text/tabwriter"
	"time"

	"github.com/mixlayer/cli/internal/client"
	"github.com/mixlayer/cli/internal/cliutil"
	"github.com/mixlayer/cli/internal/config"
	"github.com/spf13/cobra"
)

type rootFlags struct {
	asJSON       bool
	compact      bool
	csv          bool
	plain        bool
	quiet        bool
	dryRun       bool
	noCache      bool
	noInput      bool
	yes          bool
	agent        bool
	selectFields string
	configPath   string
	homePath     string
	profileName  string
	timeout      time.Duration
	rateLimit    float64
}

// RootCmd returns the Cobra command tree without executing it. The MCP server
// uses this to mirror every user-facing command as an agent tool.
func RootCmd() *cobra.Command {
	var flags rootFlags
	return newRootCmd(&flags)
}

// Execute runs the CLI in non-interactive mode: never prompts, all values via flags or stdin.
func Execute() error {
	var flags rootFlags
	rootCmd := newRootCmd(&flags)

	err := rootCmd.Execute()
	if err != nil && strings.Contains(err.Error(), "unknown flag") {
		msg := err.Error()
		// Extract the flag name from the error message (e.g., "unknown flag: --foob")
		if idx := strings.Index(msg, "unknown flag: "); idx >= 0 {
			flagStr := strings.TrimSpace(msg[idx+len("unknown flag: "):])
			if suggestion := suggestFlag(flagStr, rootCmd); suggestion != "" {
				// Cobra already printed `Error: unknown flag: --foob` before
				// returning; the wrap below attaches the hint to err.Error()
				// for downstream consumers and exit-code classification, but
				// would never reach stderr now that main.go no longer prints
				// err. Emit the hint explicitly so the suggestion still
				// shows up under Cobra's error line.
				fmt.Fprintf(os.Stderr, "hint: did you mean --%s?\n", suggestion)
				err = fmt.Errorf("%w\nhint: did you mean --%s?", err, suggestion)
			}
		}
	}
	if err != nil && isCobraUsageError(err) {
		// Cobra/pflag pre-RunE errors (unknown flag, unknown command,
		// missing required, etc.) never flow through usageErr() because
		// they originate inside rootCmd.Execute() before any user RunE
		// runs. Without this wrap, ExitCode() falls through to the
		// default and emits 1 — clobbering the conventional code-2 for
		// usage errors that the helpers.go contract already promises.
		return usageErr(err)
	}
	return err
}

// isCobraUsageError reports whether err matches one of Cobra/pflag's
// pre-RunE usage-error shapes. Detection is by message prefix to match
// the same approach the unknown-flag hint path uses above; neither
// Cobra nor pflag exports typed sentinels for these.
//
// Patterns are anchored to the literal punctuation Cobra and pflag
// emit so an application's own RunE error that happens to contain the
// substring "required flag" or "invalid argument" doesn't get
// misclassified as a usage error.
//
// Patterns covered (Cobra v1.x + pflag v1.x as of 2026-05):
//   - "unknown flag: --foo"                            (pflag)
//   - "unknown shorthand flag: 'x' in -x"              (pflag)
//   - "unknown command \"foo\" for ..."                (Cobra)
//   - "required flag \"foo\" not set"                  (Cobra, single missing)
//   - "required flag(s) \"foo\" not set"               (Cobra, multiple missing)
//   - "flag needs an argument: --foo"                  (pflag, missing value)
//   - "invalid argument \"x\" for \"--y\" flag: ..."   (pflag, parse failure)
//
// Cobra emits the singular form ("required flag") when exactly one
// MarkFlagRequired flag is missing, and the plural form ("required
// flag(s)") only when multiple are missing on the same command. Both
// shapes must be anchored to avoid matching app-level errors that
// happen to mention "required flag" as prose; the trailing space + quote
// (`required flag "`) is the literal punctuation cobra emits.
//
// Returns false for nil err.
func isCobraUsageError(err error) bool {
	if err == nil {
		return false
	}
	msg := err.Error()
	return strings.HasPrefix(msg, "unknown flag") ||
		strings.HasPrefix(msg, "unknown shorthand flag") ||
		strings.HasPrefix(msg, "unknown command") ||
		strings.HasPrefix(msg, `required flag "`) ||
		strings.HasPrefix(msg, `required flag(s) "`) ||
		strings.HasPrefix(msg, "flag needs an argument:") ||
		strings.HasPrefix(msg, `invalid argument "`)
}

func newRootCmd(flags *rootFlags) *cobra.Command {
	rootCmd := &cobra.Command{
		Use:   "mixlayer",
		Short: "Call Mixlayer models, compare them, track cost, and mask personal data",
		Long: `mixlayer is the command line for Mixlayer's OpenAI-compatible API.

Start here:
  mixlayer auth setup          get an API key
  mixlayer doctor              check your key and connection
  mixlayer models list --live-prices
  mixlayer ask "question" --model <model-id>

Compare and save:   ladder, escalate, council, compare, savings
Private data:       shield ask, shield scan, shield redact, vault
Local ledger:       grep, replay, sql
Agents:             --agent, agent setup, agent-context, which

Add --agent to any command for JSON output and no prompts.`,
		SilenceUsage: true,
		Version:      version,
	}
	rootCmd.SetVersionTemplate("mixlayer {{ .Version }}\n")

	rootCmd.PersistentFlags().BoolVar(&flags.asJSON, "json", false, "Output as JSON")
	rootCmd.PersistentFlags().BoolVar(&flags.compact, "compact", false, "Return only key fields (id, name, status, timestamps) for minimal token usage")
	rootCmd.PersistentFlags().BoolVar(&flags.csv, "csv", false, "Output as CSV (table and array responses)")
	rootCmd.PersistentFlags().BoolVar(&flags.plain, "plain", false, "Output as plain tab-separated text")
	rootCmd.PersistentFlags().BoolVar(&flags.quiet, "quiet", false, "Bare output, one value per line")
	rootCmd.PersistentFlags().StringVar(&flags.configPath, "config", "", "Config file path")
	rootCmd.PersistentFlags().StringVar(&flags.homePath, "home", "", "Root directory for config, data, state, and cache files")
	rootCmd.PersistentFlags().DurationVar(&flags.timeout, "timeout", 60*time.Second, "Request timeout")
	rootCmd.PersistentFlags().BoolVar(&flags.dryRun, "dry-run", false, "Show request without sending")
	rootCmd.PersistentFlags().BoolVar(&flags.noCache, "no-cache", false, "Bypass response cache")
	rootCmd.PersistentFlags().BoolVar(&flags.noInput, "no-input", false, "Disable all interactive prompts (for CI/agents)")
	rootCmd.PersistentFlags().StringVar(&flags.selectFields, "select", "", "Comma-separated fields to include in output (e.g. --select id,name,status)")
	rootCmd.PersistentFlags().BoolVar(&flags.yes, "yes", false, "Skip confirmation prompts (for agents and scripts)")
	rootCmd.PersistentFlags().BoolVar(&noColor, "no-color", false, "Disable colored output")
	rootCmd.PersistentFlags().BoolVar(&humanFriendly, "human-friendly", false, "Enable colored output and rich formatting")
	rootCmd.PersistentFlags().BoolVar(&flags.agent, "agent", false, "Set all agent-friendly defaults (--json --compact --no-input --no-color --yes)")
	rootCmd.PersistentFlags().StringVar(&flags.profileName, "profile", "", "Apply values from a saved profile (see 'mixlayer profile list')")
	rootCmd.PersistentFlags().Float64Var(&flags.rateLimit, "rate-limit", 0, "Max requests per second (0 to disable)")

	rootCmd.PersistentPreRunE = func(cmd *cobra.Command, args []string) error {
		if _, err := cliutil.SetHomeOverride(flags.homePath); err != nil {
			return err
		}
		if flags.profileName != "" {
			profile, err := GetProfile(flags.profileName)
			if err != nil {
				return err
			}
			if profile == nil {
				available := ListProfileNames()
				if len(available) == 0 {
					return fmt.Errorf("profile %q not found (no profiles saved yet; run '%s profile save <name> --<flag> <value>')", flags.profileName, cmd.Root().Name())
				}
				return fmt.Errorf("profile %q not found; available: %s", flags.profileName, strings.Join(available, ", "))
			}
			if err := ApplyProfileToFlags(cmd, profile); err != nil {
				return err
			}
		}
		if (flags.agent || shouldDetectAgent()) && (!cmd.Flags().Changed("human-friendly") || !humanFriendly) {
			if !cmd.Flags().Changed("json") {
				flags.asJSON = true
			}
			if !cmd.Flags().Changed("compact") {
				flags.compact = true
			}
			if !cmd.Flags().Changed("no-input") {
				flags.noInput = true
			}
			if !cmd.Flags().Changed("yes") {
				flags.yes = true
			}
			if !cmd.Flags().Changed("no-color") {
				noColor = true
			}
		}
		return nil
	}
	rootCmd.AddCommand(newModelsCmd(flags))
	rootCmd.AddCommand(newDoctorCmd(flags))
	rootCmd.AddCommand(newAuthCmd(flags))
	rootCmd.AddCommand(newAgentContextCmd(rootCmd))
	rootCmd.AddCommand(newAgentCmd(rootCmd))
	rootCmd.AddCommand(newProfileCmd(flags))
	rootCmd.AddCommand(newWhichCmd(flags))
	rootCmd.AddCommand(newChatPromotedCmd(flags))
	rootCmd.AddCommand(newAskCmd(flags))
	rootCmd.AddCommand(newReplayCmd(flags))
	rootCmd.AddCommand(newGrepCmd(flags))
	rootCmd.AddCommand(newSQLCmd(flags))
	rootCmd.AddCommand(newShieldCmd(flags))
	rootCmd.AddCommand(newVaultCmd(flags))
	rootCmd.AddCommand(newLadderCmd(flags))
	rootCmd.AddCommand(newEscalateCmd(flags))
	addExperimentalCommand(rootCmd, newDecideCmd(flags), cliutil.JiniEnabled())
	rootCmd.AddCommand(newCouncilCmd(flags))
	rootCmd.AddCommand(newSavingsCmd(flags))
	rootCmd.AddCommand(newCompareCmd(flags))
	rootCmd.AddCommand(newVersionCmd(flags))

	return rootCmd
}

func shouldDetectAgent() bool {
	if os.Getenv("MIXLAYER_NO_AGENT_DETECT") == "1" {
		return false
	}
	for _, name := range []string{"CLAUDECODE", "CLAUDE_CODE", "CODEX_SANDBOX", "CURSOR_AGENT", "GEMINI_CLI", "AI_AGENT"} {
		if os.Getenv(name) != "" {
			return true
		}
	}
	return false
}

// addExperimentalCommand registers a command that stays callable but is left
// out of help, which, agent-context and MCP unless enabled is true.
func addExperimentalCommand(root *cobra.Command, command *cobra.Command, enabled bool) {
	if !enabled {
		command.Hidden = true
		if command.Annotations == nil {
			command.Annotations = map[string]string{}
		}
		command.Annotations[experimentalAnnotation] = "true"
	}
	root.AddCommand(command)
}

const experimentalAnnotation = "cli:experimental"

func ExitCode(err error) int {
	var codeErr *cliError
	if As(err, &codeErr) {
		return codeErr.code
	}
	return 1
}

func (f *rootFlags) newClient() (*client.Client, error) {
	cfg, err := config.Load(f.configPath)
	if err != nil {
		return nil, configErr(err)
	}
	c := client.New(cfg, f.timeout, f.rateLimit)
	c.DryRun = f.dryRun
	c.NoCache = f.noCache
	return c, nil
}

func (f *rootFlags) printJSON(w *cobra.Command, v any) error {
	return printJSONFiltered(w.OutOrStdout(), v, f)
}

func (f *rootFlags) printTable(w *cobra.Command, headers []string, rows [][]string) error {
	if f.asJSON {
		return fmt.Errorf("use printJSON for JSON output")
	}
	tw := tabwriter.NewWriter(w.OutOrStdout(), 2, 4, 2, ' ', 0)
	header := ""
	for i, h := range headers {
		if i > 0 {
			header += "\t"
		}
		header += h
	}
	fmt.Fprintln(tw, header)
	for _, row := range rows {
		line := ""
		for i, cell := range row {
			if i > 0 {
				line += "\t"
			}
			line += cell
		}
		fmt.Fprintln(tw, line)
	}
	return tw.Flush()
}
