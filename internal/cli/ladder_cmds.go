// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

package cli

import (
	"context"
	"encoding/json"
	"fmt"
	"math"
	"regexp"
	"strings"

	"github.com/mixlayer/cli/internal/pricing"

	"github.com/mixlayer/cli/internal/cliutil"

	"github.com/mixlayer/cli/internal/client"
	"github.com/mixlayer/cli/internal/ladder"
	"github.com/mixlayer/cli/internal/store"
	"github.com/mixlayer/cli/internal/systemone"
	"github.com/spf13/cobra"
)

// cli:data-source live
func newLadderCmd(flags *rootFlags) *cobra.Command {
	var rungsSpec, dbPath string
	var reasoning bool
	var seed int64
	cmd := &cobra.Command{
		Use:         "ladder <question>",
		Short:       "Run one prompt across selected Mixlayer model rungs",
		Example:     `  mixlayer ladder "Which option is cheapest?" --reasoning --json`,
		Annotations: map[string]string{"mcp:hidden": "true", "cli:no-error-path-probe": "true"},
		RunE: func(cmd *cobra.Command, args []string) error {
			if len(args) == 0 {
				return cmd.Help()
			}
			if dryRunOK(flags) {
				source := "offline_fallback"
				if rungsSpec != "all" {
					source = "explicit"
				}
				return flags.printJSON(cmd, map[string]any{"dry_run": true, "endpoint": "/chat/completions", "models": ladder.Rungs(rungsSpec), "prompt": args[0], "catalog_source": source})
			}
			c, err := flags.newClient()
			if err != nil {
				return err
			}
			s, err := openMixStore(cmd.Context(), dbPath)
			if err != nil {
				return err
			}
			defer s.Close()
			catalog, _ := loadLiveCatalog(cmd.Context(), flags, s)
			var seedPtr *int64
			if seed != 0 {
				seedPtr = &seed
			}
			rungs := rungsForCatalog(rungsSpec, catalog)
			results := ladder.AskAcross(cmd.Context(), c, args[0], rungs, reasoning, seedPtr)
			applyCatalogCosts(results, catalog)
			groupID := store.NewID("ladder")
			if err := saveLadderGroup(cmd.Context(), s, "ladder", groupID, args[0], rungs, ladder.FirstConfident(results), "", seed, results); err != nil {
				return err
			}
			if wantHuman(cmd, flags) {
				renderLadderHuman(cmd.OutOrStdout(), results, ladder.FirstConfident(results))
			} else if err := outputJSON(cmd, map[string]any{"results": results, "first_confident_model": ladder.FirstConfident(results), "catalog_source": catalog.Source}); err != nil {
				return err
			}
			return ladderOutcome(results, flags)
		},
	}
	cmd.Flags().StringVar(&rungsSpec, "rungs", "all", "Comma-separated model rungs or all")
	cmd.Flags().BoolVar(&reasoning, "reasoning", false, "Request reasoning_content and compare it across rungs")
	cmd.Flags().Int64Var(&seed, "seed", 0, "Best-effort deterministic seed")
	cmd.Flags().StringVar(&dbPath, "db", "", "SQLite database file path")
	return cmd
}

func newEscalateCmd(flags *rootFlags) *cobra.Command {
	var confidence float64
	var threshold float64
	var checker, checkerModel string
	var rungsSpec, dbPath string
	cmd := &cobra.Command{
		Use:         "escalate <question>",
		Short:       "Climb the model ladder only until a rung is confident enough",
		Example:     "  mixlayer escalate \"Classify these tickets\" --confidence 0.85 --json\n  mixlayer escalate \"Explain this result\" --checker jini --checker-model <classification-model> --threshold 0.8 --agent",
		Annotations: map[string]string{"mcp:hidden": "true", "cli:no-error-path-probe": "true"},
		RunE: func(cmd *cobra.Command, args []string) error {
			if len(args) == 0 {
				return cmd.Help()
			}
			if checker != "heuristic" && checker != "jini" {
				return usageErr(fmt.Errorf("--checker must be heuristic or jini"))
			}
			if math.IsNaN(threshold) || threshold < 0 || threshold > 1 {
				return usageErr(fmt.Errorf("--threshold must be between 0 and 1"))
			}
			var c *client.Client
			var err error
			if checker == "jini" {
				c, checkerModel, err = (decideOptions{model: checkerModel}).client(flags)
				if err != nil {
					return fmt.Errorf("jini checker: %w", err)
				}
				if err := jiniCheckerRequest(checkerModel, args[0], "pending answer").Validate(); err != nil {
					return usageErr(err)
				}
			}
			if dryRunOK(flags) {
				source := "offline_fallback"
				if rungsSpec != "all" {
					source = "explicit"
				}
				preview := map[string]any{"dry_run": true, "endpoint": "/chat/completions", "models": ladder.Rungs(rungsSpec), "prompt": args[0], "catalog_source": source, "checker": checker}
				if checker == "jini" {
					preview["checker_url"], _ = client.SystemOneURL(c.BaseURL)
					preview["checker_body_template"] = jiniCheckerRequest(checkerModel, args[0], "<rung response>")
					preview["threshold"] = threshold
				}
				return flags.printJSON(cmd, preview)
			}
			if c == nil {
				c, err = flags.newClient()
			}
			if err != nil {
				return err
			}
			s, err := openMixStore(cmd.Context(), dbPath)
			if err != nil {
				return err
			}
			defer s.Close()
			catalog, _ := loadLiveCatalog(cmd.Context(), flags, s)
			configuredRungs := rungsForCatalog(rungsSpec, catalog)
			var results []ladder.Result
			var checkerErr error
			for _, rung := range configuredRungs {
				prompt := args[0]
				if checker == "heuristic" {
					prompt += "\n\nReturn a concise answer. End with exactly one line: CONFIDENCE: high, medium, or low. Use low or medium when uncertain."
				}
				part := ladder.AskAcross(cmd.Context(), c, prompt, []string{rung}, false, nil)
				applyCatalogCosts(part, catalog)
				stop := false
				for i := range part {
					part[i].CheckerResult = &ladder.CheckerResult{Checker: checker, CheckerModel: checkerModel}
					if part[i].Error != "" || part[i].Answer == "" {
						continue
					}
					if checker == "heuristic" {
						score := heuristicConfidence(part[i].Answer)
						part[i].CheckerScore = &score
						stop = score >= confidence
					} else {
						ctx, cancel := boundCtx(cmd.Context(), flags)
						checked, err := c.SystemOne(ctx, jiniCheckerRequest(checkerModel, args[0], part[i].Answer))
						cancel()
						if err != nil {
							checkerErr = fmt.Errorf("jini checker failed after rung %s: %w", rung, decisionAPIError(err))
							part[i].CheckerError = checkerErr.Error()
						} else {
							part[i].CheckerScore = checked.Answers["correct"].Noul
							part[i].CheckerUsage = checked.Usage
							stop = *part[i].CheckerScore >= threshold
						}
					}
				}
				results = append(results, part...)
				if stop || checkerErr != nil {
					break
				}
			}
			total := 0.0
			// Classification prices are not guaranteed in the generation catalog.
			// Avoid presenting generation-only totals as the whole checked climb.
			unknownTotal := checker == "jini"
			for _, r := range results {
				if r.CostUnknown {
					unknownTotal = true
				} else {
					total += r.CostUSD
				}
			}
			triedRungs, escalationModel := escalationLedgerMetadata(configuredRungs, results)
			groupID := store.NewID("ladder")
			if err := saveLadderGroup(cmd.Context(), s, "escalate", groupID, args[0], triedRungs, escalationModel, "", 0, results); err != nil {
				return err
			}
			var baseline, saved, totalOutput any
			var baselineReason string
			var costReason string
			if checker == "jini" {
				costReason = "classification checker cost is not included in the generation catalog"
			}
			if !unknownTotal {
				totalOutput = pricing.RoundUSD(total)
			}
			if ladder.FirstConfident(results) == "" {
				baselineReason = "no rung returned an answer"
			} else if frontier := catalog.frontier(); frontier == "" {
				baselineReason = "frontier model is unpriced"
			} else {
				failedBaseline := false
				for _, result := range results {
					if result.Model == frontier && result.Error != "" {
						failedBaseline = true
					}
				}
				if failedBaseline {
					baselineReason = "frontier baseline model failed"
				} else if price, ok := catalog.price(frontier, 1000, 1000); ok {
					baseline = price
					if !unknownTotal {
						saved = pricing.RoundUSD(price - total)
					}
				} else {
					baselineReason = "frontier model is unpriced"
				}
			}
			if wantHuman(cmd, flags) {
				renderEscalateHuman(cmd.OutOrStdout(), results, totalOutput, baseline, saved, baselineReason)
			} else if err := outputJSON(cmd, map[string]any{"group_id": groupID, "results": results, "cost_usd": totalOutput, "cost_reason": costReason, "frontier_baseline_usd": baseline, "saved_usd": saved, "baseline_reason": baselineReason, "catalog_source": catalog.Source}); err != nil {
				return err
			}
			if checkerErr != nil {
				return checkerErr
			}
			return ladderOutcome(results, flags)
		},
	}
	cmd.Flags().Float64Var(&confidence, "confidence", 0.85, "Confidence threshold")
	cmd.Flags().StringVar(&checker, "checker", "heuristic", "Answer checker: heuristic or jini (checker errors stop the climb)")
	cmd.Flags().StringVar(&checkerModel, "checker-model", "", "Jini classification model (falls back to config classification_model)")
	cmd.Flags().Float64Var(&threshold, "threshold", 0.8, "Minimum Jini noul score to stop (0–1)")
	cmd.Flags().StringVar(&rungsSpec, "rungs", "all", "Comma-separated model rungs or all")
	cmd.Flags().StringVar(&dbPath, "db", "", "SQLite database file path")
	if !cliutil.JiniEnabled() {
		cmd.Example = "  mixlayer escalate \"Classify these tickets\" --confidence 0.85 --json"
		cmd.Flags().Lookup("checker").Usage = "Answer checker"
		_ = cmd.Flags().MarkHidden("checker-model")
		_ = cmd.Flags().MarkHidden("threshold")
	}
	return cmd
}

func jiniCheckerRequest(model, prompt, answer string) systemone.Request {
	return systemone.Request{Model: model, State: systemone.JSON(map[string]string{"request": prompt, "response": answer}), Questions: systemone.Questions{
		"correct": {Type: "noul", Instructions: systemone.JSON("Does the response fully and correctly answer the request?"), Criteria: systemone.JSON(map[string]string{
			"true":  "The response is correct, complete, relevant, and satisfies all requirements of the request.",
			"false": "The response is incorrect, incomplete, irrelevant, unsupported, or fails a requirement of the request.",
		})},
	}}
}

func escalationLedgerMetadata(configuredRungs []string, results []ladder.Result) ([]string, string) {
	triedCount := len(results)
	if triedCount > len(configuredRungs) {
		triedCount = len(configuredRungs)
	}
	triedRungs := append([]string(nil), configuredRungs[:triedCount]...)
	if len(results) == 0 {
		return triedRungs, ""
	}
	return triedRungs, results[len(results)-1].Model
}

func newCouncilCmd(flags *rootFlags) *cobra.Command {
	var membersSpec, judge, dbPath string
	cmd := &cobra.Command{
		Use:         "council <question>",
		Short:       "Fan out to several rungs and ask a judge model to synthesize",
		Example:     `  mixlayer council "Pick the safest launch plan" --json`,
		Annotations: map[string]string{"mcp:hidden": "true", "cli:no-error-path-probe": "true"},
		RunE: func(cmd *cobra.Command, args []string) error {
			if len(args) == 0 {
				return cmd.Help()
			}
			if dryRunOK(flags) {
				return flags.printJSON(cmd, map[string]any{"dry_run": true, "endpoint": "/chat/completions", "models": append(ladder.Rungs(membersSpec), judge), "prompt": args[0], "catalog_source": "offline_fallback"})
			}
			c, err := flags.newClient()
			if err != nil {
				return err
			}
			s, err := openMixStore(cmd.Context(), dbPath)
			if err != nil {
				return err
			}
			defer s.Close()
			catalog, _ := loadLiveCatalog(cmd.Context(), flags, s)
			members := rungsForCatalog(membersSpec, catalog)
			if !cmd.Flags().Changed("judge") {
				judge = catalog.defaultModel()
			}
			results := ladder.AskAcross(cmd.Context(), c, args[0], members, true, nil)
			applyCatalogCosts(results, catalog)
			var b strings.Builder
			fmt.Fprintf(&b, "Question: %s\n\nSynthesize these model answers and flag disagreements:\n", args[0])
			for i, r := range results {
				appendCouncilMemberPrompt(&b, i+1, r)
			}
			judgeResult := ladder.AskAcross(cmd.Context(), c, b.String(), []string{judge}, false, nil)
			applyCatalogCosts(judgeResult, catalog)
			groupID := store.NewID("ladder")
			if err := s.SaveLadder(cmd.Context(), groupID, args[0], append(append([]string{}, members...), judge), "", judge); err != nil {
				return err
			}
			if err := saveLadderRuns(cmd.Context(), s, "council", groupID, args[0], 0, results); err != nil {
				return err
			}
			if err := saveLadderRuns(cmd.Context(), s, "council-judge", groupID, b.String(), 0, judgeResult); err != nil {
				return err
			}
			if wantHuman(cmd, flags) {
				renderCouncilHuman(cmd.OutOrStdout(), results, judgeResult)
			} else if err := outputJSON(cmd, map[string]any{"group_id": groupID, "members": results, "judge": judgeResult, "catalog_source": catalog.Source}); err != nil {
				return err
			}
			return ladderOutcome(append(results, judgeResult...), flags)
		},
	}
	cmd.Flags().StringVar(&membersSpec, "members", "all", "Comma-separated member models or all active models")
	cmd.Flags().StringVar(&judge, "judge", defaultFrontierModel, "Judge model")
	autoModelFlag(cmd, "judge", "Judge model (default: the highest-priced active model in your live catalog)")
	cmd.Flags().StringVar(&dbPath, "db", "", "SQLite database file path")
	return cmd
}

func ladderOutcome(results []ladder.Result, flags *rootFlags) error {
	failed := 0
	var first error
	for _, result := range results {
		if result.Error != "" || result.Answer == "" {
			failed++
			if first == nil {
				first = result.Cause()
				if first == nil {
					first = fmt.Errorf("model %s returned no answer", result.Model)
				}
			}
		}
	}
	if failed == 0 {
		return nil
	}
	if failed == len(results) {
		return classifyAPIError(first, flags)
	}
	return partialFailureErr(fmt.Errorf("%d of %d model calls failed", failed, len(results)))
}

func saveLadderGroup(ctx context.Context, s *store.Store, command, groupID, prompt string, rungs []string, firstConfident, judge string, seed int64, results []ladder.Result) error {
	if err := s.SaveLadder(ctx, groupID, prompt, rungs, firstConfident, judge); err != nil {
		return err
	}
	return saveLadderRuns(ctx, s, command, groupID, prompt, seed, results)
}

func saveLadderRuns(ctx context.Context, s *store.Store, command, groupID, prompt string, seed int64, results []ladder.Result) error {
	for _, res := range results {
		raw, _ := json.Marshal(res)
		if err := s.SaveRun(ctx, store.RunRecord{
			ID:               store.NewID("run"),
			GroupID:          groupID,
			Command:          command,
			Prompt:           prompt,
			Answer:           res.Answer,
			Reasoning:        res.Reasoning,
			Model:            res.Model,
			Seed:             seed,
			RawJSON:          raw,
			PromptTokens:     res.PromptTokens,
			CompletionTokens: res.CompletionTokens,
			TotalTokens:      res.TotalTokens,
			CostUSD:          res.CostUSD,
			CostUnknown:      res.CostUnknown || res.Error != "",
			LatencyMS:        res.LatencyMS,
		}); err != nil {
			return err
		}
	}
	return nil
}

func appendCouncilMemberPrompt(b *strings.Builder, index int, r ladder.Result) {
	payload := struct {
		Model     string `json:"model"`
		Reasoning string `json:"reasoning,omitempty"`
		Answer    string `json:"answer"`
	}{
		Model:     r.Model,
		Reasoning: r.Reasoning,
		Answer:    r.Answer,
	}
	encoded, _ := json.Marshal(payload)
	fmt.Fprintf(b, "\n---BEGIN MEMBER %d---\n%s\n---END MEMBER %d---\n", index, encoded, index)
}

func heuristicConfidence(answer string) float64 {
	if strings.TrimSpace(answer) == "" {
		return 0
	}
	lower := strings.ToLower(answer)
	if strings.Contains(lower, "not sure") || strings.Contains(lower, "uncertain") || strings.Contains(lower, "can't determine") || strings.Contains(lower, "cannot determine") {
		return 0.4
	}
	// Hedging in the answer outranks a claimed "high". Otherwise use the
	// model's explicit self-report. Models often put the
	// line inline ("... 13. CONFIDENCE: high"), so take the last occurrence
	// anywhere rather than requiring its own line.
	if m := confidenceRe.FindAllStringSubmatch(answer, -1); len(m) > 0 {
		switch strings.ToLower(m[len(m)-1][1]) {
		case "high":
			return 0.9
		case "medium":
			return 0.7
		default:
			return 0.4
		}
	}
	return 0.65
}

var confidenceRe = regexp.MustCompile(`(?i)\bconfidence\s*[:=]\s*\**\s*(high|medium|low)\b`)
