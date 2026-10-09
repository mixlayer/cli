// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

package cli

import (
	"bytes"
	"strings"
	"testing"

	"github.com/mixlayer/cli/internal/ladder"
)

func TestFormatUSD(t *testing.T) {
	cases := map[float64]string{0: "$0", 0.0000042000000000000004: "$0.0000042", 0.00505: "$0.00505", 1.25: "$1.25"}
	for in, want := range cases {
		if got := formatUSD(in); got != want {
			t.Errorf("formatUSD(%v) = %q, want %q", in, got, want)
		}
	}
}

func TestRenderEscalateHuman(t *testing.T) {
	noColor = true
	defer func() { noColor = false }()
	low, high := 0.4, 0.9
	results := []ladder.Result{
		{Model: "cheap/a", Answer: "maybe", TotalTokens: 10, LatencyMS: 400, CheckerResult: &ladder.CheckerResult{Checker: "heuristic", CheckerScore: &low}},
		{Model: "mid/b", Answer: "391", TotalTokens: 12, LatencyMS: 650, CostUSD: 0.0000039, CheckerResult: &ladder.CheckerResult{Checker: "heuristic", CheckerScore: &high}},
	}
	var b bytes.Buffer
	renderEscalateHuman(&b, results, 0.0000039, 0.00505, 0.0050461, "")
	out := b.String()
	for _, want := range []string{"cheap/a", "confidence 0.40, escalating", "mid/b", "confidence 0.90, stopped here", "  391", "Spent $0.0000039", "would have cost $0.00505", "saved $0.0050461"} {
		if !strings.Contains(out, want) {
			t.Errorf("missing %q in:\n%s", want, out)
		}
	}
	if strings.Contains(out, "{") {
		t.Errorf("human view printed JSON:\n%s", out)
	}
}

func TestFormatMaskedCounts(t *testing.T) {
	n, s := formatMaskedCounts(map[string]int{"EMAIL": 1, "PERSON": 2, "PHONE": 0})
	if n != 3 || s != "2 PERSON, 1 EMAIL" {
		t.Fatalf("got %d %q", n, s)
	}
}

func TestStripConfidenceTag(t *testing.T) {
	cases := map[string]string{
		"No, 91 = 7*13. CONFIDENCE: high":  "No, 91 = 7*13.",
		"Yes.\n**CONFIDENCE: low**":        "Yes.",
		"391":                              "391",
		"CONFIDENCE: is a word I like\nok": "CONFIDENCE: is a word I like\nok",
	}
	for in, want := range cases {
		if got := stripConfidenceTag(in); got != want {
			t.Errorf("stripConfidenceTag(%q) = %q, want %q", in, got, want)
		}
	}
}
