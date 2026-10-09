// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

package ladder

import (
	"context"
	"encoding/json"
	"strings"
	"sync"

	"github.com/mixlayer/cli/internal/client"
	"github.com/mixlayer/cli/internal/mixapi"
	"github.com/mixlayer/cli/internal/pricing"
	"github.com/mixlayer/cli/internal/systemone"
)

type Result struct {
	Model            string  `json:"model"`
	Answer           string  `json:"answer,omitempty"`
	Reasoning        string  `json:"reasoning,omitempty"`
	PromptTokens     int     `json:"prompt_tokens,omitempty"`
	CacheReadTokens  int     `json:"cache_read_tokens,omitempty"`
	CompletionTokens int     `json:"completion_tokens,omitempty"`
	TotalTokens      int     `json:"total_tokens,omitempty"`
	UsageReported    bool    `json:"usage_reported"`
	CostUSD          float64 `json:"-"`
	CostUnknown      bool    `json:"-"`
	LatencyMS        int64   `json:"latency_ms,omitempty"`
	Error            string  `json:"error,omitempty"`
	*CheckerResult
	cause error
}

// CheckerResult is embedded only for escalation. It is also persisted in the
// existing run raw_json column, so adding checker metadata needs no migration.
type CheckerResult struct {
	Checker      string           `json:"checker"`
	CheckerScore *float64         `json:"checker_score"`
	CheckerModel string           `json:"checker_model"`
	CheckerError string           `json:"checker_error,omitempty"`
	CheckerUsage *systemone.Usage `json:"checker_usage,omitempty"`
}

func (r Result) Cause() error { return r.cause }

func (r Result) MarshalJSON() ([]byte, error) {
	type plain Result
	var cost *float64
	if !r.CostUnknown && r.Error == "" {
		cost = &r.CostUSD
	}
	return json.Marshal(struct {
		plain
		CostUSD *float64 `json:"cost_usd"`
	}{plain(r), cost})
}

func Rungs(spec string) []string {
	if strings.TrimSpace(spec) == "" || strings.EqualFold(spec, "all") {
		return append([]string(nil), pricing.DefaultRungs...)
	}
	parts := strings.Split(spec, ",")
	out := make([]string, 0, len(parts))
	for _, p := range parts {
		p = strings.TrimSpace(p)
		if p != "" {
			out = append(out, p)
		}
	}
	return out
}

func AskAcross(ctx context.Context, c *client.Client, prompt string, rungs []string, reasoning bool, seed *int64) []Result {
	if len(rungs) == 0 {
		rungs = pricing.DefaultRungs
	}
	results := make([]Result, len(rungs))
	var wg sync.WaitGroup
	for i, model := range rungs {
		wg.Add(1)
		go func(i int, model string) {
			defer wg.Done()
			thinking := reasoning
			req := mixapi.ChatRequest{
				Model:    model,
				Messages: []mixapi.Message{{Role: "user", Content: prompt}},
				Thinking: &thinking,
				Seed:     seed,
			}
			res, err := mixapi.Chat(ctx, c, req)
			results[i].Model = model
			if err != nil {
				results[i].Error = err.Error()
				results[i].cause = err
				return
			}
			results[i].Answer = res.Answer
			results[i].Reasoning = res.Reasoning
			results[i].PromptTokens = res.PromptTokens
			results[i].CacheReadTokens = res.CacheReadTokens
			results[i].CompletionTokens = res.CompletionTokens
			results[i].TotalTokens = res.TotalTokens
			results[i].UsageReported = res.UsageReported
			results[i].LatencyMS = res.LatencyMS
			results[i].CostUnknown = true
		}(i, model)
	}
	wg.Wait()
	return results
}

func FirstConfident(results []Result) string {
	for _, r := range results {
		if r.Error == "" && strings.TrimSpace(r.Answer) != "" {
			return r.Model
		}
	}
	return ""
}
