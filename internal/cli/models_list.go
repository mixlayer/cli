// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

package cli

import (
	"encoding/json"
	"fmt"
	"os"

	"github.com/spf13/cobra"
)

func newModelsListCmd(flags *rootFlags) *cobra.Command {
	var dbPath string
	var livePrices bool

	cmd := &cobra.Command{
		Use:         "list",
		Short:       "List all available models",
		Example:     "  mixlayer models list",
		Annotations: map[string]string{"cli:endpoint": "models.list", "cli:method": "GET", "cli:path": "/models", "mcp:read-only": "true"},
		RunE: func(cmd *cobra.Command, args []string) error {
			if livePrices {
				if flags.dryRun {
					return flags.printJSON(cmd, map[string]any{"endpoint": "/v1/organizations/{org}/models", "fallback_endpoint": "/v1/models", "dry_run": true})
				}
				s, err := openMixStore(cmd.Context(), dbPath)
				if err != nil {
					return err
				}
				defer s.Close()
				catalog, fetchErr := loadLiveCatalog(cmd.Context(), flags, s)
				if flags.asJSON || flags.agent {
					result := map[string]any{"models": catalog.Models, "source": catalog.Source}
					if fetchErr != nil {
						result["warning"] = fetchErr.Error()
					}
					return flags.printJSON(cmd, result)
				}
				rows := make([][]string, 0, len(catalog.Models))
				for _, model := range catalog.Models {
					price := "unknown"
					if model.PriceMOutputTokensUSD != nil {
						price = fmt.Sprintf("%.4f", *model.PriceMOutputTokensUSD)
					}
					rows = append(rows, []string{model.ModelSKU, model.Status, price})
				}
				if fetchErr != nil {
					fmt.Fprintf(cmd.ErrOrStderr(), "warning: %v\n", fetchErr)
				}
				return flags.printTable(cmd, []string{"MODEL", "STATUS", "OUTPUT USD/1M"}, rows)
			}
			c, err := flags.newClient()
			if err != nil {
				return err
			}

			path := "/models"
			params := map[string]string{}
			data, prov, err := liveRead(cmd.Context(), c, path, params)
			if err != nil {
				return classifyAPIError(err, flags)
			}
			// Print provenance to stderr for human-facing output only.
			// Machine-format flags (--json, --csv, --compact, --quiet, --plain,
			// --select) and piped stdout suppress this line; the JSON envelope
			// already carries meta.source for those consumers.
			// Keep this gate aligned with the other endpoint commands.
			if wantsHumanTable(cmd.OutOrStdout(), flags) {
				var countItems []json.RawMessage
				_ = json.Unmarshal(data, &countItems)
				printProvenance(cmd, len(countItems), prov)
			}
			// For JSON output, wrap with provenance envelope before passing through flags.
			// --select wins over --compact when both are set; --compact only runs when
			// no explicit fields were requested. Explicit format flags (--csv, --quiet,
			// --plain) opt out of the auto-JSON path so piped consumers that asked for
			// a non-JSON format reach the standard pipeline below.
			if flags.asJSON || (!isTerminal(cmd.OutOrStdout()) && !flags.csv && !flags.quiet && !flags.plain) {
				filtered := data
				if flags.selectFields != "" {
					filtered = filterFields(filtered, flags.selectFields)
				} else if flags.compact {
					filtered = compactFields(filtered)
				}
				wrapped, wrapErr := wrapWithProvenance(filtered, prov)
				if wrapErr != nil {
					return wrapErr
				}
				return printOutput(cmd.OutOrStdout(), wrapped, true)
			}
			// For all other output modes (table, csv, plain, quiet), use the standard pipeline
			if wantsHumanTable(cmd.OutOrStdout(), flags) {
				var items []map[string]any
				if json.Unmarshal(data, &items) == nil && len(items) > 0 {
					if err := printAutoTable(cmd.OutOrStdout(), items); err != nil {
						return err
					}
					if len(items) >= 25 {
						fmt.Fprintf(os.Stderr, "\nShowing %d results. To narrow: add --limit, --json --select, or filter flags.\n", len(items))
					}
					return nil
				}
			}
			return printOutputWithFlags(cmd.OutOrStdout(), data, flags)
		},
	}
	cmd.Flags().BoolVar(&livePrices, "live-prices", false, "Show cached live organization prices")
	cmd.Flags().StringVar(&dbPath, "db", "", "SQLite database file path for live catalog cache")

	return cmd
}
