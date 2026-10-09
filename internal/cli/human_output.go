// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

package cli

import (
	"fmt"
	"io"
	"sort"
	"strconv"
	"strings"

	"github.com/mixlayer/cli/internal/ladder"
	"github.com/mixlayer/cli/internal/pricing"
	"github.com/mixlayer/cli/internal/store"
	"github.com/spf13/cobra"
)

// wantHuman reports whether a command should print its readable terminal
// view instead of JSON: stdout is a terminal and neither --json nor agent
// mode is on. Pipes, files and agents always get JSON.
func wantHuman(cmd *cobra.Command, flags *rootFlags) bool {
	if flags == nil || flags.asJSON || flags.agent {
		return false
	}
	return isTerminal(cmd.OutOrStdout())
}

// formatUSD prints a dollar amount without float noise or scientific
// notation: "$0" for free, "$0.0000039" for tiny amounts.
func formatUSD(v float64) string {
	v = pricing.RoundUSD(v)
	if v == 0 {
		return "$0"
	}
	return "$" + strconv.FormatFloat(v, 'f', -1, 64)
}

func resultCost(r ladder.Result) string {
	if r.CostUnknown {
		return "cost unknown"
	}
	return formatUSD(r.CostUSD)
}

func formatLatency(ms int64) string {
	if ms <= 0 {
		return ""
	}
	return fmt.Sprintf("%.2fs", float64(ms)/1000)
}

// indentAnswer trims the answer and indents each line by two spaces.
func indentAnswer(s string) string {
	s = strings.TrimSpace(s)
	if s == "" {
		return "  " + dim("(empty answer)")
	}
	lines := strings.Split(s, "\n")
	for i, l := range lines {
		lines[i] = "  " + l
	}
	return strings.Join(lines, "\n")
}

func dim(s string) string {
	if !colorEnabled() {
		return s
	}
	return "\033[2m" + s + "\033[0m"
}

func magenta(s string) string {
	if !colorEnabled() {
		return s
	}
	return "\033[35m" + s + "\033[0m"
}

// writeResultBlock prints one model's answer under a header line with its
// latency, tokens and cost.
func writeResultBlock(w io.Writer, r ladder.Result, note string) {
	meta := []string{}
	if l := formatLatency(r.LatencyMS); l != "" {
		meta = append(meta, l)
	}
	if r.TotalTokens > 0 {
		meta = append(meta, fmt.Sprintf("%d tokens", r.TotalTokens))
	}
	meta = append(meta, resultCost(r))
	header := bold(r.Model) + "  " + dim(strings.Join(meta, " · "))
	if note != "" {
		header += "  " + note
	}
	fmt.Fprintln(w, header)
	if r.Error != "" {
		fmt.Fprintln(w, "  "+red("error: "+r.Error))
	} else {
		fmt.Fprintln(w, indentAnswer(stripConfidenceTag(r.Answer)))
	}
	fmt.Fprintln(w)
}

func sumCost(results []ladder.Result) (float64, bool) {
	var total float64
	for _, r := range results {
		if r.CostUnknown {
			return 0, false
		}
		total += r.CostUSD
	}
	return total, true
}

func renderLadderHuman(w io.Writer, results []ladder.Result, firstConfident string) {
	for _, r := range results {
		note := ""
		if r.Model == firstConfident {
			note = green("first confident answer")
		}
		writeResultBlock(w, r, note)
	}
	if total, ok := sumCost(results); ok {
		fmt.Fprintf(w, "%s across %d models\n", bold(formatUSD(total)), len(results))
	}
}

func renderEscalateHuman(w io.Writer, results []ladder.Result, cost, baseline, saved any, baselineReason string) {
	for i, r := range results {
		note := ""
		if r.CheckerResult != nil && r.CheckerScore != nil {
			note = fmt.Sprintf("confidence %.2f", *r.CheckerScore)
			if i == len(results)-1 && r.Error == "" {
				note = green(note + ", stopped here")
			} else {
				note = yellow(note + ", escalating")
			}
		}
		writeResultBlock(w, r, note)
	}
	if c, ok := cost.(float64); ok {
		line := "Spent " + bold(formatUSD(c))
		if b, ok := baseline.(float64); ok {
			line += " · the top model would have cost " + formatUSD(b)
			if s, ok := saved.(float64); ok && s > 0 {
				line += " · " + green("saved "+formatUSD(s))
			}
		} else if baselineReason != "" {
			line += dim(" (" + baselineReason + ")")
		}
		fmt.Fprintln(w, line)
	}
}

func renderCouncilHuman(w io.Writer, members, judge []ladder.Result) {
	for _, r := range members {
		writeResultBlock(w, r, "")
	}
	for _, j := range judge {
		fmt.Fprintln(w, magenta("Judge"))
		writeResultBlock(w, j, "")
	}
	if total, ok := sumCost(append(append([]ladder.Result{}, members...), judge...)); ok {
		fmt.Fprintf(w, "%s total\n", bold(formatUSD(total)))
	}
}

func runAsResult(r store.RunRecord) ladder.Result {
	return ladder.Result{Model: r.Model, Answer: r.Answer, TotalTokens: r.TotalTokens, LatencyMS: r.LatencyMS, CostUSD: r.CostUSD, CostUnknown: r.CostUnknown}
}

func renderCompareHuman(w io.Writer, cheap, frontier store.RunRecord, delta any, note string) {
	writeResultBlock(w, runAsResult(cheap), dim("cheap"))
	writeResultBlock(w, runAsResult(frontier), dim("frontier"))
	if d, ok := delta.(float64); ok {
		fmt.Fprintf(w, "The frontier answer cost %s more.\n", bold(formatUSD(d)))
	}
	if note != "" {
		fmt.Fprintln(w, dim(note))
	}
}

func renderSavingsHuman(w io.Writer, runs, unknown, tokens int, actual, baseline float64, vs string) {
	fmt.Fprintf(w, "%s  %d runs, %d tokens\n", bold("Your usage"), runs, tokens)
	label := "at " + vs + " prices"
	width := len(label)
	fmt.Fprintf(w, "  %-*s  %s\n", width, "you paid", formatUSD(actual))
	fmt.Fprintf(w, "  %-*s  %s\n", width, label, formatUSD(baseline))
	fmt.Fprintf(w, "  %-*s  %s\n", width, "saved", green(formatUSD(baseline-actual)))
	if unknown > 0 {
		fmt.Fprintln(w, dim(fmt.Sprintf("  %d runs with unknown cost were left out", unknown)))
	}
}

// formatMaskedCounts renders {"EMAIL":1,"PERSON":2} as "2 PERSON, 1 EMAIL".
func formatMaskedCounts(counts map[string]int) (int, string) {
	type kv struct {
		k string
		v int
	}
	var items []kv
	total := 0
	for k, v := range counts {
		if v > 0 {
			items = append(items, kv{k, v})
			total += v
		}
	}
	sort.Slice(items, func(i, j int) bool {
		if items[i].v != items[j].v {
			return items[i].v > items[j].v
		}
		return items[i].k < items[j].k
	})
	parts := make([]string, len(items))
	for i, it := range items {
		parts[i] = fmt.Sprintf("%d %s", it.v, it.k)
	}
	return total, strings.Join(parts, ", ")
}

// stripConfidenceTag removes the trailing "CONFIDENCE: <level>" marker that
// escalate asks models to add; the score is already shown in the header.
func stripConfidenceTag(s string) string {
	t := strings.TrimRight(s, " \n\t*")
	i := strings.LastIndex(strings.ToUpper(t), "CONFIDENCE:")
	if i < 0 || strings.Contains(t[i:], "\n") || len(t)-i > 24 {
		return s
	}
	return strings.TrimRight(strings.TrimRight(t[:i], " \t*"), "\n")
}
