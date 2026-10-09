// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

package cli

import (
	"fmt"

	"github.com/mixlayer/cli/internal/pricing"
	"github.com/spf13/cobra"
)

// cli:data-source auto
func newSavingsCmd(flags *rootFlags) *cobra.Command {
	var dbPath, vs string
	cmd := &cobra.Command{
		Use:         "savings",
		Short:       "Roll up Mixlayer ledger cost against a frontier baseline",
		Example:     `  mixlayer savings --vs gpt-frontier --json`,
		Annotations: map[string]string{"mcp:read-only": "true"},
		RunE: func(cmd *cobra.Command, args []string) error {
			s, err := openMixStore(cmd.Context(), dbPath)
			if err != nil {
				return err
			}
			defer s.Close()
			runs, err := s.RecentRuns(cmd.Context(), 10000)
			if err != nil {
				return err
			}
			var actual, baseline float64
			var tokens, unknown int
			for _, r := range runs {
				if r.CostUnknown {
					unknown++
					continue
				}
				actual += r.CostUSD
				tokens += r.TotalTokens
				baseline += pricing.Baseline(vs, r.TotalTokens)
			}
			if wantHuman(cmd, flags) {
				renderSavingsHuman(cmd.OutOrStdout(), len(runs)-unknown, unknown, tokens, actual, baseline, vs)
				return nil
			}
			return outputJSON(cmd, map[string]any{"runs": len(runs) - unknown, "excluded_unknown_cost_runs": unknown, "tokens": tokens, "actual_usd": pricing.RoundUSD(actual), "baseline_usd": pricing.RoundUSD(baseline), "saved_usd": pricing.RoundUSD(baseline - actual), "baseline": vs})
		},
	}
	cmd.Flags().StringVar(&dbPath, "db", "", "SQLite database file path")
	cmd.Flags().StringVar(&vs, "vs", "gpt-frontier", "Baseline: gpt-frontier or claude-frontier")
	return cmd
}

func newCompareCmd(flags *rootFlags) *cobra.Command {
	var cheap, frontier, dbPath string
	cmd := &cobra.Command{
		Use:         "compare <question>",
		Short:       "Compare a cheap rung with the frontier and price the delta",
		Example:     `  mixlayer compare "Summarize this incident" --cheap qwen/qwen3.5-9b --json`,
		Annotations: map[string]string{"mcp:hidden": "true", "cli:no-error-path-probe": "true"},
		RunE: func(cmd *cobra.Command, args []string) error {
			if len(args) == 0 {
				return cmd.Help()
			}
			if dryRunOK(flags) {
				source := "offline_fallback"
				if cmd.Flags().Changed("cheap") && cmd.Flags().Changed("frontier") {
					source = "explicit"
				}
				return flags.printJSON(cmd, map[string]any{"dry_run": true, "endpoint": "/chat/completions", "models": []string{cheap, frontier}, "prompt": args[0], "catalog_source": source})
			}
			s, err := openMixStore(cmd.Context(), dbPath)
			if err != nil {
				return err
			}
			defer s.Close()
			catalog, _ := loadLiveCatalog(cmd.Context(), flags, s)
			if !cmd.Flags().Changed("frontier") {
				frontier = catalog.defaultModel()
			}
			if !cmd.Flags().Changed("cheap") {
				if rungs := catalog.activeRungs(); len(rungs) > 0 {
					cheap = rungs[0]
				}
			}
			cheapRun, err := chatAndSave(cmd.Context(), flags, s, "compare", args[0], cheap, 0, false)
			if err != nil {
				return err
			}
			frontierRun, err := chatAndSave(cmd.Context(), flags, s, "compare", args[0], frontier, 0, false)
			if err != nil {
				return err
			}
			note := "No prior ladder history found for an evidence-grounded quality rate."
			history, _ := s.SearchRuns(cmd.Context(), args[0], 50)
			if len(history) > 2 {
				note = fmt.Sprintf("Evidence note: %d related saved runs exist in your local ledger for this prompt pattern.", len(history))
			}
			var delta any
			if !frontierRun.CostUnknown && !cheapRun.CostUnknown {
				delta = pricing.RoundUSD(frontierRun.CostUSD - cheapRun.CostUSD)
			}
			if wantHuman(cmd, flags) {
				renderCompareHuman(cmd.OutOrStdout(), cheapRun, frontierRun, delta, note)
				return nil
			}
			return outputJSON(cmd, map[string]any{
				"cheap": cheapRun, "frontier": frontierRun,
				"delta_usd":    delta,
				"quality_note": note,
			})
		},
	}
	cmd.Flags().StringVar(&cheap, "cheap", "qwen/qwen3.5-9b", "Cheap rung")
	cmd.Flags().StringVar(&frontier, "frontier", defaultFrontierModel, "Frontier rung")
	autoModelFlag(cmd, "cheap", "Cheap model (default: the cheapest active model in your live catalog)")
	autoModelFlag(cmd, "frontier", "Frontier model (default: the highest-priced active model in your live catalog)")
	cmd.Flags().StringVar(&dbPath, "db", "", "SQLite database file path")
	return cmd
}
