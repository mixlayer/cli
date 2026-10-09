// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

package pricing

import (
	"math"
	"strings"
)

type Price struct {
	InputPerMillion  float64 `json:"input_per_million"`
	OutputPerMillion float64 `json:"output_per_million"`
}

type ModelInfo struct {
	ID                string `json:"id"`
	ContextWindow     int    `json:"context_window"`
	SupportsTools     bool   `json:"supports_tools"`
	SupportsReasoning bool   `json:"supports_reasoning"`
}

var Mixlayer = map[string]Price{
	"qwen/qwen3.5-4b-free": {InputPerMillion: 0, OutputPerMillion: 0},
	"qwen/qwen3.5-9b":      {InputPerMillion: 0.10, OutputPerMillion: 0.40},
	"qwen/qwen3.6-35b-a3b": {InputPerMillion: 0.25, OutputPerMillion: 1.30},
	"qwen/qwen3.8-27b":     {InputPerMillion: 0.25, OutputPerMillion: 1.95},
	"z-ai/glm-5.3":         {InputPerMillion: 1.15, OutputPerMillion: 3.90},
	"gpt-frontier":         {InputPerMillion: 5.00, OutputPerMillion: 15.00},
	"claude-frontier":      {InputPerMillion: 3.00, OutputPerMillion: 15.00},
}

var KnownModels = []ModelInfo{
	{ID: "qwen/qwen3.5-4b-free", ContextWindow: 131072, SupportsTools: true, SupportsReasoning: true},
	{ID: "qwen/qwen3.5-9b", ContextWindow: 131072, SupportsTools: true, SupportsReasoning: true},
	{ID: "qwen/qwen3.6-35b-a3b", ContextWindow: 131072, SupportsTools: true, SupportsReasoning: true},
	{ID: "qwen/qwen3.8-27b", ContextWindow: 131072, SupportsTools: true, SupportsReasoning: true},
	{ID: "z-ai/glm-5.3", ContextWindow: 262144, SupportsTools: true, SupportsReasoning: true},
}

var DefaultRungs = []string{
	"qwen/qwen3.5-4b-free",
	"qwen/qwen3.5-9b",
	"qwen/qwen3.6-35b-a3b",
	"qwen/qwen3.8-27b",
	"z-ai/glm-5.3",
}

func Estimate(model string, promptTokens, completionTokens int) float64 {
	p, ok := Mixlayer[strings.TrimSpace(model)]
	if !ok {
		return 0
	}
	return RoundUSD(float64(promptTokens)/1_000_000*p.InputPerMillion + float64(completionTokens)/1_000_000*p.OutputPerMillion)
}

func Baseline(vs string, totalTokens int) float64 {
	key := strings.TrimSpace(vs)
	if key == "" {
		key = "gpt-frontier"
	}
	p, ok := Mixlayer[key]
	if !ok {
		p = Mixlayer["gpt-frontier"]
	}
	half := totalTokens / 2
	return EstimateWithPrice(p, half, totalTokens-half)
}

func EstimateWithPrice(p Price, promptTokens, completionTokens int) float64 {
	return RoundUSD(float64(promptTokens)/1_000_000*p.InputPerMillion + float64(completionTokens)/1_000_000*p.OutputPerMillion)
}

// RoundUSD rounds a dollar amount to 1e-12 so float error such as
// 0.0000042000000000000004 never reaches output or the ledger. Per-token
// prices are far coarser than this, so no real precision is lost.
func RoundUSD(v float64) float64 {
	return math.Round(v*1e12) / 1e12
}
