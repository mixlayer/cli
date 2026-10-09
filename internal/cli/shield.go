// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

package cli

import (
	"context"
	"fmt"
	"net/url"
	"os"
	"strings"
	"time"

	"github.com/mixlayer/cli/internal/shield"
	"github.com/mixlayer/cli/internal/store"
	"github.com/mixlayer/cli/internal/upstream"
	"github.com/spf13/cobra"
)

// cli:data-source auto
func newShieldCmd(flags *rootFlags) *cobra.Command {
	cmd := &cobra.Command{
		Use:   "shield",
		Short: "Privacy firewall for masking data before frontier-model calls",
		RunE:  parentNoSubcommandRunE(flags),
	}
	cmd.AddCommand(newShieldScanCmd(flags))
	cmd.AddCommand(newShieldRedactCmd(flags))
	cmd.AddCommand(newShieldRestructureCmd(flags))
	cmd.AddCommand(newShieldIngestCmd(flags))
	cmd.AddCommand(newShieldAskCmd(flags))
	cmd.AddCommand(newShieldDictCmd(flags))
	cmd.AddCommand(newShieldEvalCmd(flags))
	cmd.AddCommand(newShieldAuditCmd(flags))
	return cmd
}

func newShieldScanCmd(flags *rootFlags) *cobra.Command {
	var maxRisk int
	var dbPath string
	var detector detectorFlags
	cmd := &cobra.Command{
		Use:         "scan <file>",
		Short:       "Detect PII locally without an upstream call",
		Example:     `  mixlayer shield scan customer-export.csv --max-risk 0 --json`,
		Annotations: map[string]string{"mcp:read-only": "true", "cli:typed-exit-codes": "0,6"},
		RunE: func(cmd *cobra.Command, args []string) error {
			if len(args) == 0 {
				return cmd.Help()
			}
			text, err := readRequiredFile(args[0])
			if err != nil {
				return err
			}
			runtime, err := detector.runtime(cmd, flags)
			if err != nil {
				return err
			}
			s, err := openMixStore(cmd.Context(), dbPath)
			if err != nil {
				return err
			}
			defer s.Close()
			terms, err := s.DictionaryTerms(cmd.Context())
			if err != nil {
				return err
			}
			entities, _, err := shield.DetectWith(cmd.Context(), text, terms, runtime.Call, runtime.Fallback)
			if err != nil {
				return classifyAPIError(err, flags)
			}
			result := map[string]any{"risk": shield.RiskScore(entities), "entities": entities}
			if err := outputJSON(cmd, result); err != nil {
				return err
			}
			if shield.RiskScore(entities) > maxRisk {
				return partialFailureErr(fmt.Errorf("PII risk %d exceeds --max-risk %d", shield.RiskScore(entities), maxRisk))
			}
			return nil
		},
	}
	cmd.Flags().IntVar(&maxRisk, "max-risk", 0, "Exit non-zero when the per-entity max risk exceeds this value (not volume-weighted)")
	cmd.Flags().StringVar(&dbPath, "db", "", "SQLite database file path for local dictionary")
	detector.bind(cmd)
	return cmd
}

func newShieldRedactCmd(flags *rootFlags) *cobra.Command {
	var dbPath, output string
	var diff bool
	var maskColumns []string
	var detector detectorFlags
	cmd := &cobra.Command{
		Use:         "redact <file>",
		Short:       "Mask a file and populate the local vault",
		Example:     `  mixlayer shield redact customer-export.csv --diff -o masked.csv`,
		Annotations: map[string]string{"mcp:hidden": "true", "cli:no-error-path-probe": "true"},
		RunE: func(cmd *cobra.Command, args []string) error {
			if len(args) == 0 {
				return cmd.Help()
			}
			s, err := openMixStore(cmd.Context(), dbPath)
			if err != nil {
				return err
			}
			defer s.Close()
			text, err := readRequiredFile(args[0])
			if err != nil {
				return err
			}
			runtime, err := detector.runtime(cmd, flags)
			if err != nil {
				return err
			}
			if runtime.ExtraTerms, err = csvColumnTerms(text, maskColumns); err != nil {
				return err
			}
			res, _, err := detectAndRedact(cmd.Context(), s, text, diff, runtime)
			if err != nil {
				return classifyAPIError(err, flags)
			}
			if output != "" {
				return os.WriteFile(output, []byte(res.Text), 0o600)
			}
			if diff || flags.asJSON || flags.agent {
				return outputJSON(cmd, res)
			}
			fmt.Fprint(cmd.OutOrStdout(), res.Text)
			return nil
		},
	}
	cmd.Flags().StringVarP(&output, "output", "o", "", "Write masked copy to file")
	cmd.Flags().StringVar(&dbPath, "db", "", "SQLite database file path")
	cmd.Flags().BoolVar(&diff, "diff", false, "Include tokenization details")
	cmd.Flags().StringSliceVar(&maskColumns, "mask-columns", nil, "For CSV files: mask every value in these columns (comma-separated header names)")
	detector.bind(cmd)
	return cmd
}

func newShieldRestructureCmd(flags *rootFlags) *cobra.Command {
	var output, coarsenDates string
	var bucket bool
	var drop []string
	cmd := &cobra.Command{
		Use:         "restructure <file>",
		Short:       "Coarsen values to reduce re-identification risk",
		Example:     `  mixlayer shield restructure customers.csv --bucket --coarsen-dates month --drop email -o safe.csv`,
		Annotations: map[string]string{"mcp:hidden": "true"},
		RunE: func(cmd *cobra.Command, args []string) error {
			if len(args) == 0 {
				return cmd.Help()
			}
			text, err := readRequiredFile(args[0])
			if err != nil {
				return err
			}
			out, err := shield.Restructure(text, shield.RestructureOptions{BucketNumerics: bucket, CoarsenDates: coarsenDates, DropColumns: drop})
			if err != nil {
				return err
			}
			if output != "" {
				return os.WriteFile(output, []byte(out), 0o600)
			}
			fmt.Fprint(cmd.OutOrStdout(), out)
			return nil
		},
	}
	cmd.Flags().BoolVar(&bucket, "bucket", false, "Bucket numeric values into ranges")
	cmd.Flags().StringVar(&coarsenDates, "coarsen-dates", "", "Coarsen ISO dates to month or quarter")
	cmd.Flags().StringSliceVar(&drop, "drop", nil, "CSV column to drop (repeatable)")
	cmd.Flags().StringVarP(&output, "output", "o", "", "Write restructured copy to file")
	return cmd
}

func newShieldIngestCmd(flags *rootFlags) *cobra.Command {
	var dbPath, output, manifestPath string
	var maxBytes int
	var detector detectorFlags
	cmd := &cobra.Command{
		Use:         "ingest <bigfile>",
		Short:       "Split, redact, and reassemble a large corpus with one shared vault",
		Example:     `  mixlayer shield ingest big-export.csv -o masked-corpus.csv --manifest tranches.json`,
		Annotations: map[string]string{"mcp:hidden": "true"},
		RunE: func(cmd *cobra.Command, args []string) error {
			if len(args) == 0 {
				return cmd.Help()
			}
			s, err := openMixStore(cmd.Context(), dbPath)
			if err != nil {
				return err
			}
			defer s.Close()
			text, err := readRequiredFile(args[0])
			if err != nil {
				return err
			}
			tranches, err := shield.SplitRecords(text, maxBytes)
			if err != nil {
				return err
			}
			var b strings.Builder
			runtime, err := detector.runtime(cmd, flags)
			if err != nil {
				return err
			}
			manifest := shield.TrancheManifest{}
			for _, tr := range tranches {
				res, _, err := detectAndRedact(cmd.Context(), s, tr.Text, false, runtime)
				if err != nil {
					return classifyAPIError(err, flags)
				}
				b.WriteString(res.Text)
				kinds := map[string]int{}
				for _, e := range res.Entities {
					kinds[e.Kind]++
				}
				manifest.Tranches = append(manifest.Tranches, shield.TrancheSummary{
					Index: tr.Index, FromLine: tr.FromLine, ToLine: tr.ToLine,
					EntityCount: len(res.Entities), EntityKinds: kinds,
				})
			}
			if manifestPath != "" {
				if err := os.WriteFile(manifestPath, shield.ManifestJSON(manifest), 0o600); err != nil {
					return err
				}
			}
			return writeShieldIngestResult(cmd, flags, output, b.String(), map[string]any{"tranches": len(tranches), "manifest": manifest, "output": output})
		},
	}
	cmd.Flags().StringVar(&dbPath, "db", "", "SQLite database file path")
	cmd.Flags().StringVarP(&output, "output", "o", "", "Write masked corpus to file")
	cmd.Flags().StringVar(&manifestPath, "manifest", "", "Write tranche manifest to file")
	cmd.Flags().IntVar(&maxBytes, "max-bytes", 64000, "Maximum bytes per tranche")
	detector.bind(cmd)
	return cmd
}

func writeShieldIngestResult(cmd *cobra.Command, flags *rootFlags, output, corpus string, summary map[string]any) error {
	if output != "" {
		if err := os.WriteFile(output, []byte(corpus), 0o600); err != nil {
			return err
		}
		return outputJSON(cmd, summary)
	}
	if flags.asJSON || flags.agent {
		summary["masked_corpus"] = corpus
		return outputJSON(cmd, summary)
	}
	fmt.Fprint(cmd.OutOrStdout(), corpus)
	return nil
}

func newShieldAskCmd(flags *rootFlags) *cobra.Command {
	var dbPath, dataFile, model, guard, upstreamName, upstreamBaseURL, upstreamKeyEnv string
	var showThinking, showSent, force, acceptRegexOnly bool
	var maskColumns []string
	var detector detectorFlags
	cmd := &cobra.Command{
		Use:         "ask <question>",
		Short:       "Ask a frontier model using only masked local data",
		Example:     `  mixlayer shield ask "What segments are at risk?" --data customer-export.csv --json`,
		Annotations: map[string]string{"mcp:hidden": "true"},
		RunE: func(cmd *cobra.Command, args []string) error {
			if len(args) == 0 {
				return cmd.Help()
			}
			cfg, err := upstream.Resolve(upstreamName, upstreamBaseURL, upstreamKeyEnv)
			if err != nil {
				return usageErr(err)
			}
			if upstreamName != "mixlayer" && (!cmd.Flags().Changed("model") || strings.TrimSpace(model) == "") {
				return usageErr(fmt.Errorf("--model is required for --upstream %s", upstreamName))
			}
			if detector.Name == "regex" && upstreamName != "mixlayer" && !acceptRegexOnly {
				return usageErr(fmt.Errorf("regex detection misses names, addresses and international IDs; use --detector local:<model> or mixlayer:<model> (or pass --accept-regex-only)"))
			}
			warnings := []string{}
			if detector.Name == "regex" && upstreamName == "mixlayer" {
				warnings = append(warnings, "regex detection misses names, addresses and international IDs; use --detector local:<model> or mixlayer:<model>")
				if !flags.asJSON && !flags.agent {
					fmt.Fprintln(cmd.ErrOrStderr(), "warning: "+warnings[0])
				}
			}
			if upstreamName == "mixlayer" {
				c, err := flags.newClient()
				if err != nil {
					return err
				}
				cfg.BaseURL = c.RequestBaseURL()
				u, err := url.Parse(cfg.BaseURL)
				if err != nil {
					return err
				}
				cfg.Host = u.Host
			}
			if flags.dryRun && detector.Name != "regex" {
				return usageErr(fmt.Errorf("--dry-run requires --detector regex because model detection makes a network call"))
			}
			s, err := openMixStore(cmd.Context(), dbPath)
			if err != nil {
				return err
			}
			defer s.Close()
			if upstreamName == "mixlayer" && !cmd.Flags().Changed("model") && !flags.dryRun {
				catalog, _ := loadLiveCatalog(cmd.Context(), flags, s)
				model = catalog.defaultModel()
			}
			data := ""
			if dataFile != "" {
				if dataFile == "-" {
					data, err = readTextArg(dataFile)
				} else {
					data, err = readRequiredFile(dataFile)
				}
				if err != nil {
					return err
				}
			}
			runtime, err := detector.runtime(cmd, flags)
			if err != nil {
				return err
			}
			if runtime.ExtraTerms, err = csvColumnTerms(data, maskColumns); err != nil {
				return err
			}
			prepared, err := prepareShieldAskPayloadWith(cmd.Context(), s, data, args[0], runtime)
			if err != nil {
				return classifyAPIError(err, flags)
			}
			if flags.dryRun {
				preview := map[string]any{"dry_run": true, "destination_host": cfg.Host, "model": model, "payload": prepared.Payload,
					"messages":      []map[string]string{{"role": "system", "content": placeholderInstruction}, {"role": "user", "content": prepared.Payload}},
					"masked_counts": prepared.MaskedCounts, "tripwire_hits": prepared.Leaks, "tripwire_detector": runtime.Name, "warnings": warnings}
				if upstreamName == "mixlayer" {
					preview["catalog_source"] = "offline_fallback"
					if cmd.Flags().Changed("model") {
						preview["catalog_source"] = "explicit"
					}
				}
				if flags.asJSON || flags.agent {
					return flags.printJSON(cmd, preview)
				}
				fmt.Fprintf(cmd.OutOrStdout(), "Destination host: %s\nModel: %s\nSystem: %s\nPayload:\n%s\nMasked counts: %v\nTripwire hits: %d\n", cfg.Host, model, placeholderInstruction, prepared.Payload, prepared.MaskedCounts, len(prepared.Leaks))
				return nil
			}
			if len(prepared.Leaks) > 0 && !force {
				return partialFailureErr(fmt.Errorf("masked payload still contains %d detector hits; use --force to send anyway", len(prepared.Leaks)))
			}
			var detectorCost *float64
			detectorCostUnknown := false
			if strings.HasPrefix(runtime.Name, "mixlayer:") && runtime.Usage != nil {
				catalog, _ := loadLiveCatalog(cmd.Context(), flags, s)
				detectorModel := strings.TrimPrefix(runtime.Name, "mixlayer:")
				if cost, ok := catalog.priceWithCache(detectorModel, runtime.Usage.PromptTokens, runtime.Usage.CompletionTokens, runtime.Usage.CacheReadTokens); ok && (!runtime.Usage.UnknownUsage || catalog.free(detectorModel)) {
					detectorCost = &cost
				} else {
					detectorCostUnknown = true
				}
			}
			run, err := saveShieldChat(cmd.Context(), flags, s, cfg, model, prepared.Payload, showThinking, detectorCost, detectorCostUnknown)
			if err != nil {
				return err
			}
			restored, err := shield.RehydrateCheckedForTokens(cmd.Context(), s, run.Answer, prepared.Tokens)
			if err != nil {
				return err
			}
			rehydratedRun, err := rehydrateRunForOutputWithTokens(cmd.Context(), s, run, restored.Text, prepared.Tokens)
			if err != nil {
				return err
			}
			audit := store.AuditRecord{
				ID: store.NewID("audit"), RunID: run.ID, Command: "shield ask",
				CreatedAt:     time.Now().UTC(),
				PayloadSHA256: sha256Hex([]byte(prepared.Payload)), ByteCount: len(prepared.Payload), Model: model, GuardModel: guard, UpstreamHost: cfg.Host,
				Detector: runtime.Name, TripwireDetector: runtime.Name, DetectorHost: runtime.Host, DetectorNotice: runtime.Notice, MaskedCounts: prepared.MaskedCounts,
				IgnoredModelSpans: ignoredSpanCount(runtime),
				DetectorCostUSD:   detectorCost,
				MaskedEntities:    prepared.MaskedEntities, LeakedPIICount: len(prepared.Leaks), RestoredCount: restored.RestoredCount,
				UnrestoredCount: restored.UnrestoredCount, UnrestoredTokens: restored.UnrestoredTokens, Repairs: restored.Repairs,
			}
			if prepared.Fallback {
				audit.DetectorFallback = "regex"
			}
			if !run.CostUnknown {
				audit.CostUSD = &run.CostUSD
			}
			if err := s.SaveAudit(cmd.Context(), audit); err != nil {
				return err
			}
			if flags.asJSON || flags.agent {
				return flags.printJSON(cmd, map[string]any{"answer": restored.Text, "run": rehydratedRun, "audit": audit, "warnings": warnings,
					"restored_count": restored.RestoredCount, "unrestored_count": restored.UnrestoredCount, "unrestored_tokens": restored.UnrestoredTokens, "repairs": restored.Repairs})
			}
			fmt.Fprintln(cmd.OutOrStdout(), restored.Text)
			if restored.UnrestoredCount > 0 {
				fmt.Fprintf(cmd.ErrOrStderr(), "warning: %d placeholder-like tokens could not be restored: %s\n", restored.UnrestoredCount, strings.Join(restored.UnrestoredTokens, ", "))
			}
			if showSent {
				fmt.Fprintf(cmd.ErrOrStderr(), "%s\n%s\n", dim("sent to "+cfg.Host+":"), dim(prepared.Payload))
			}
			if n, kinds := formatMaskedCounts(prepared.MaskedCounts); n > 0 {
				fmt.Fprintf(cmd.ErrOrStderr(), "%s %s\n", green(fmt.Sprintf("masked %d before sending (%s), restored %d locally", n, kinds, restored.RestoredCount)), dim("· receipt "+audit.ID))
			} else {
				fmt.Fprintf(cmd.ErrOrStderr(), "%s %s\n", yellow("nothing to mask"), dim("· receipt "+audit.ID))
			}
			return nil
		},
	}
	cmd.Flags().StringVar(&dataFile, "data", "", "Local data file, or - for stdin (optional)")
	cmd.Flags().StringVar(&model, "model", defaultFrontierModel, "Frontier model")
	autoModelFlag(cmd, "model", "Model that answers the masked question (default on Mixlayer: the highest-priced active model in your live catalog; required for other upstreams)")
	cmd.Flags().StringVar(&upstreamName, "upstream", "mixlayer", "Upstream: mixlayer, openai, anthropic, openrouter, or custom")
	cmd.Flags().StringVar(&upstreamBaseURL, "upstream-base-url", "", "Base URL for custom upstream")
	cmd.Flags().StringVar(&upstreamKeyEnv, "upstream-key-env", "", "Environment variable name holding upstream key")
	cmd.Flags().StringVar(&guard, "guard", defaultGuardModel, "Guard model label recorded in receipts; local detectors enforce the tripwire")
	cmd.Flags().BoolVar(&showThinking, "show-thinking", false, "Request reasoning_content")
	cmd.Flags().BoolVar(&showSent, "show-sent", false, "Print the masked prompt exactly as it was sent")
	cmd.Flags().StringSliceVar(&maskColumns, "mask-columns", nil, "With a CSV --data file: mask every value in these columns, e.g. customer,email,company")
	cmd.Flags().BoolVar(&force, "force", false, "Send even if the outbound tripwire detects residual PII")
	cmd.Flags().BoolVar(&acceptRegexOnly, "accept-regex-only", false, "Allow regex-only detection with an external upstream")
	cmd.Flags().StringVar(&dbPath, "db", "", "SQLite database file path")
	detector.bind(cmd)
	return cmd
}

type shieldAskPayload struct {
	Payload        string
	MaskedEntities int
	MaskedCounts   map[string]int
	Leaks          []shield.Entity
	Fallback       bool
	Tokens         []string
}

func prepareShieldAskPayload(ctx context.Context, s *store.Store, data, question string) (shieldAskPayload, error) {
	return prepareShieldAskPayloadWith(ctx, s, data, question, detectorRuntime{Name: "regex"})
}

func prepareShieldAskPayloadWith(ctx context.Context, s *store.Store, data, question string, detector detectorRuntime) (shieldAskPayload, error) {
	redactedData, fallbackData, err := detectAndRedact(ctx, s, data, false, detector)
	if err != nil {
		return shieldAskPayload{}, err
	}
	redactedQuestion, fallbackQuestion, err := detectAndRedact(ctx, s, question, false, detector)
	if err != nil {
		return shieldAskPayload{}, err
	}
	payload := "Question: " + redactedQuestion.Text
	if data != "" {
		payload = fmt.Sprintf("Use this masked corpus:\n\n%s\n\nQuestion: %s", redactedData.Text, redactedQuestion.Text)
	}
	counts := map[string]int{}
	var tokens []string
	for _, e := range redactedData.Entities {
		counts[e.Kind]++
		tokens = append(tokens, e.Token)
	}
	for _, e := range redactedQuestion.Entities {
		counts[e.Kind]++
		tokens = append(tokens, e.Token)
	}
	tripwirePayload := payload
	for _, token := range tokens {
		tripwirePayload = strings.ReplaceAll(tripwirePayload, token, strings.Repeat(" ", len(token)))
	}
	terms, err := s.DictionaryTerms(ctx)
	if err != nil {
		return shieldAskPayload{}, err
	}
	terms = append(terms, detector.ExtraTerms...)
	leaks, tripwireFallback, ignored, err := shield.DetectWithStats(ctx, tripwirePayload, terms, detector.Call, detector.Fallback)
	if detector.IgnoredSpans != nil {
		*detector.IgnoredSpans += ignored
	}
	if err != nil {
		return shieldAskPayload{}, err
	}
	if leaks == nil {
		leaks = []shield.Entity{}
	}
	return shieldAskPayload{
		Payload:        payload,
		MaskedEntities: len(redactedData.Entities) + len(redactedQuestion.Entities),
		MaskedCounts:   counts,
		Leaks:          leaks,
		Fallback:       fallbackData || fallbackQuestion || tripwireFallback,
		Tokens:         tokens,
	}, nil
}

func ignoredSpanCount(detector detectorRuntime) int {
	if detector.IgnoredSpans == nil {
		return 0
	}
	return *detector.IgnoredSpans
}

func rehydrateRunForOutput(ctx context.Context, s *store.Store, run store.RunRecord, answer string) (store.RunRecord, error) {
	rehydrated := run
	rehydrated.Answer = answer
	if rehydrated.Reasoning != "" {
		reasoning, err := shield.Rehydrate(ctx, s, rehydrated.Reasoning)
		if err != nil {
			return store.RunRecord{}, err
		}
		rehydrated.Reasoning = reasoning
	}
	return rehydrated, nil
}

func rehydrateRunForOutputWithTokens(ctx context.Context, s *store.Store, run store.RunRecord, answer string, tokens []string) (store.RunRecord, error) {
	rehydrated := run
	rehydrated.Answer = answer
	if run.Reasoning != "" {
		reasoning, err := shield.RehydrateCheckedForTokens(ctx, s, run.Reasoning, tokens)
		if err != nil {
			return store.RunRecord{}, err
		}
		rehydrated.Reasoning = reasoning.Text
	}
	return rehydrated, nil
}

func newShieldAuditCmd(flags *rootFlags) *cobra.Command {
	var dbPath string
	var limit int
	cmd := &cobra.Command{
		Use:         "audit [id]",
		Short:       "Show privacy receipts",
		Example:     `  mixlayer shield audit --limit 25 --json`,
		Annotations: map[string]string{"mcp:read-only": "true"},
		RunE: func(cmd *cobra.Command, args []string) error {
			s, err := openMixStore(cmd.Context(), dbPath)
			if err != nil {
				return err
			}
			defer s.Close()
			id := ""
			if len(args) > 0 {
				id = args[0]
			}
			rows, err := s.AuditRecords(cmd.Context(), id, limit)
			if err != nil {
				return err
			}
			if rows == nil {
				rows = []store.AuditRecord{}
			}
			return outputJSON(cmd, rows)
		},
	}
	cmd.Flags().StringVar(&dbPath, "db", "", "SQLite database file path")
	cmd.Flags().IntVar(&limit, "limit", 50, "Maximum receipts")
	return cmd
}
