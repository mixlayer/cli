// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

package cli

import (
	"fmt"
	"io"
	"os"
	"strings"

	"github.com/mixlayer/cli/internal/cliutil"
	"github.com/mixlayer/cli/internal/config"
	"github.com/spf13/cobra"
)

func newAuthCmd(flags *rootFlags) *cobra.Command {
	cmd := &cobra.Command{
		Use:   "auth",
		Short: "Manage authentication for Mixlayer",
		RunE:  parentNoSubcommandRunE(flags),
	}

	cmd.AddCommand(newAuthSetupCmd(flags))
	cmd.AddCommand(newAuthStatusCmd(flags))
	cmd.AddCommand(newAuthSetTokenCmd(flags))
	cmd.AddCommand(newAuthLogoutCmd(flags))

	return cmd
}

// newAuthSetupCmd prints concrete steps for getting a credential. Side-effect
// rule: print by default, --launch opt-in to open the URL, short-circuit when
// the verifier is running this in a sandboxed subprocess.
func newAuthSetupCmd(_ *rootFlags) *cobra.Command {
	var launch bool
	cmd := &cobra.Command{
		Use:     "setup",
		Short:   "Print steps for obtaining a credential (use --launch to open the URL)",
		Example: "  mixlayer auth setup\n  mixlayer auth setup --launch",
		RunE: func(cmd *cobra.Command, args []string) error {
			w := cmd.OutOrStdout()
			fmt.Fprintln(w, "1. Create an API key at "+apiKeysURL)
			fmt.Fprintln(w, "   (new here? sign up at "+signUpURL+")")
			fmt.Fprintln(w, "2. Put it in your environment:")
			fmt.Fprintln(w, "     export MIXLAYER_API_KEY=\"your-key\"")
			fmt.Fprintln(w, "3. Check it works:")
			fmt.Fprintln(w, "     mixlayer doctor")
			if !launch || cliutil.IsVerifyEnv() {
				return nil
			}
			if err := openURL(apiKeysURL); err != nil {
				fmt.Fprintf(cmd.ErrOrStderr(), "could not open a browser: %v\n", err)
			}
			return nil
		},
	}
	cmd.Flags().BoolVar(&launch, "launch", false, "Open the setup URL in your default browser")
	return cmd
}

func newAuthStatusCmd(flags *rootFlags) *cobra.Command {
	return &cobra.Command{
		Use:     "status",
		Short:   "Show authentication status",
		Example: "  mixlayer auth status",
		RunE: func(cmd *cobra.Command, args []string) error {
			cfg, err := config.Load(flags.configPath)
			if err != nil {
				return configErr(err)
			}

			w := cmd.OutOrStdout()
			header := cfg.AuthHeader()
			authed := header != ""
			// JSON envelope: {authenticated, verified, source, config}. When not
			// authenticated, write the envelope first then return authErr
			// so exit code carries the auth-failure signal.
			if flags.asJSON {
				out := map[string]any{
					"authenticated": authed,
					"verified":      false,
					"source":        cfg.AuthSource,
					"config":        cfg.Path,
				}
				if printErr := printJSONFiltered(w, out, flags); printErr != nil {
					return printErr
				}
				if !authed {
					return authErr(fmt.Errorf("no credentials configured"))
				}
				return nil
			}
			if !authed {
				fmt.Fprintln(w, red("Not authenticated"))
				fmt.Fprintln(w, "")
				fmt.Fprintln(w, "Set your API key:")
				fmt.Fprintln(w, "  export MIXLAYER_API_KEY=\"your-key\"")
				fmt.Fprintln(w, "Create one at "+apiKeysURL)
				return authErr(fmt.Errorf("no credentials configured"))
			}

			fmt.Fprintln(w, green("Credentials present (not verified)"))
			fmt.Fprintf(w, "  Source: %s\n", cfg.AuthSource)
			fmt.Fprintf(w, "  Config: %s\n", cfg.Path)
			return nil
		},
	}
}

func newAuthSetTokenCmd(flags *rootFlags) *cobra.Command {
	return &cobra.Command{
		Use:   "set-token [token|-]",
		Short: "Save an API key to the credentials file",
		Long: `Save an API key to the local credentials file (mode 0600).

Pipe the key on stdin so it stays out of shell history and the process list:

  pbpaste | mixlayer auth set-token
  mixlayer auth set-token - < key.txt

Passing the key as an argument also works but is not recommended.`,
		Example: "  pbpaste | mixlayer auth set-token",
		Args:    cobra.MaximumNArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {
			token := ""
			if len(args) == 1 && args[0] != "-" {
				token = strings.TrimSpace(args[0])
			} else {
				if f, ok := cmd.InOrStdin().(*os.File); ok && isTerminal(f) {
					return usageErr(fmt.Errorf("pipe the key on stdin, for example: pbpaste | mixlayer auth set-token"))
				}
				b, err := io.ReadAll(io.LimitReader(cmd.InOrStdin(), 64*1024))
				if err != nil {
					return fmt.Errorf("reading key from stdin: %w", err)
				}
				token = strings.TrimSpace(string(b))
			}
			if token == "" || strings.ContainsAny(token, " \t\r\n") {
				return usageErr(fmt.Errorf("expected a single API key with no spaces"))
			}
			cfg, err := config.Load(flags.configPath)
			if err != nil {
				return configErr(err)
			}

			// Clear any legacy auth_header so AuthHeader() falls through to
			// the newly-saved credential. Without this, a pre-existing
			// auth_header value (common after regenerate) shadows the saved
			// token and set-token silently has no effect. Silent clear (no
			// log line): a masked-tail variant could leak token bytes through
			// scripted runs that capture stderr.
			cfg.AuthHeaderVal = ""
			if err := cfg.SaveTokens("", "", token, "", cfg.TokenExpiry); err != nil {
				return configErr(fmt.Errorf("saving token: %w", err))
			}

			savePath := credentialSavePath(cfg)
			// JSON envelope: {saved, config_path, credentials_path}.
			if flags.asJSON {
				out := map[string]any{
					"saved":            true,
					"config_path":      cfg.Path,
					"credentials_path": savePath,
				}
				return printJSONFiltered(cmd.OutOrStdout(), out, flags)
			}
			fmt.Fprintf(cmd.OutOrStdout(), "Token saved to %s\n", savePath)
			return nil
		},
	}
}

func credentialSavePath(cfg *config.Config) string {
	if path, err := cliutil.CredentialsFilePath(); err == nil {
		return path
	}
	if cfg != nil {
		return cfg.Path
	}
	return ""
}

func newAuthLogoutCmd(flags *rootFlags) *cobra.Command {
	return &cobra.Command{
		Use:     "logout",
		Short:   "Clear stored credentials",
		Example: "  mixlayer auth logout",
		RunE: func(cmd *cobra.Command, args []string) error {
			cfg, err := config.Load(flags.configPath)
			if err != nil {
				return configErr(err)
			}

			if err := cfg.ClearTokens(); err != nil {
				return configErr(fmt.Errorf("clearing tokens: %w", err))
			}

			// Identify which (if any) auth env var is still exported so the
			// JSON envelope and the human prose can both surface it.
			envStillSet := ""
			if envStillSet == "" && os.Getenv("MIXLAYER_API_KEY") != "" {
				envStillSet = "MIXLAYER_API_KEY"
			}

			// JSON envelope: {cleared: true, note?: "<env_var> env var is still set"}.
			if flags.asJSON {
				out := map[string]any{"cleared": true}
				if envStillSet != "" {
					out["note"] = envStillSet + " env var is still set"
				}
				return printJSONFiltered(cmd.OutOrStdout(), out, flags)
			}

			if envStillSet != "" {
				fmt.Fprintf(cmd.OutOrStdout(), "Config cleared. Note: %s env var is still set.\n", envStillSet)
				return nil
			}
			fmt.Fprintln(cmd.OutOrStdout(), "Logged out. Credentials cleared.")
			return nil
		},
	}
}
