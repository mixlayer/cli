// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

package cli

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"os"
	"strings"

	"github.com/mixlayer/cli/internal/mixapi"
	"github.com/mixlayer/cli/internal/store"
	"github.com/spf13/cobra"
)

const (
	defaultFrontierModel = "z-ai/glm-5.3"
	defaultGuardModel    = "qwen/qwen3.5-4b-free"
)

func openMixStore(ctx context.Context, dbPath string) (*store.Store, error) {
	if dbPath == "" {
		dbPath = defaultDBPath("mixlayer")
	}
	return store.OpenWithContext(ctx, dbPath)
}

func outputJSON(cmd *cobra.Command, v any) error {
	raw, err := json.MarshalIndent(v, "", "  ")
	if err != nil {
		return err
	}
	raw = append(raw, '\n')
	return printOutput(cmd.OutOrStdout(), raw, true)
}

func readTextArg(pathOrText string) (string, error) {
	if pathOrText == "-" {
		raw, err := os.ReadFile("/dev/stdin")
		return string(raw), err
	}
	if raw, err := os.ReadFile(pathOrText); err == nil {
		return string(raw), nil
	}
	return pathOrText, nil
}

func readRequiredFile(path string) (string, error) {
	if strings.TrimSpace(path) == "" {
		return "", fmt.Errorf("file path is required")
	}
	raw, err := os.ReadFile(path)
	if err != nil {
		return "", err
	}
	return string(raw), nil
}

func sha256Hex(data []byte) string {
	sum := sha256.Sum256(data)
	return hex.EncodeToString(sum[:])
}

func chatAndSave(ctx context.Context, flags *rootFlags, s *store.Store, command, prompt, model string, seed int64, reasoning bool) (store.RunRecord, error) {
	c, err := flags.newClient()
	if err != nil {
		return store.RunRecord{}, err
	}
	thinking := reasoning
	var seedPtr *int64
	if seed != 0 {
		seedPtr = &seed
	}
	res, err := mixapi.Chat(ctx, c, mixapi.ChatRequest{
		Model:    model,
		Messages: []mixapi.Message{{Role: "user", Content: prompt}},
		Thinking: &thinking,
		Seed:     seedPtr,
	})
	if err != nil {
		return store.RunRecord{}, classifyAPIError(err, flags)
	}
	catalog, _ := loadLiveCatalog(ctx, flags, s)
	cost, priced := catalog.priceWithCache(model, res.PromptTokens, res.CompletionTokens, res.CacheReadTokens)
	priced = priced && (res.UsageReported || catalog.free(model))
	params, _ := json.Marshal(map[string]any{"model": model, "seed": seed, "reasoning": reasoning})
	run := store.RunRecord{
		ID:               store.NewID("run"),
		Command:          command,
		Prompt:           prompt,
		Answer:           res.Answer,
		Reasoning:        res.Reasoning,
		Model:            model,
		Seed:             seed,
		ParamsJSON:       params,
		RawJSON:          res.Raw,
		PromptTokens:     res.PromptTokens,
		CompletionTokens: res.CompletionTokens,
		TotalTokens:      res.TotalTokens,
		CostUSD:          cost,
		CostUnknown:      !priced,
		CatalogSource:    catalog.Source,
		LatencyMS:        res.LatencyMS,
	}
	if err := s.SaveRun(ctx, run); err != nil {
		return store.RunRecord{}, err
	}
	return run, nil
}

// autoModelFlag documents a model flag whose real default comes from the live
// catalog at run time. The Go-level default stays as an offline fallback but
// is not shown in help, where a fixed model ID would go stale.
func autoModelFlag(cmd *cobra.Command, name, usage string) {
	if f := cmd.Flags().Lookup(name); f != nil {
		f.DefValue = ""
		f.Usage = usage
	}
}
