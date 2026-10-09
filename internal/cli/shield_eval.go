// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

package cli

import (
	"bufio"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"time"

	"github.com/mixlayer/cli/internal/shield"
	"github.com/mixlayer/cli/internal/store"
	"github.com/spf13/cobra"
)

type evalGold struct {
	Kind string `json:"kind"`
	Text string `json:"text"`
}
type evalSample struct {
	ID       string     `json:"id"`
	Text     string     `json:"text"`
	Entities []evalGold `json:"entities"`
}
type evalCounts struct {
	Gold     int `json:"gold"`
	Found    int `json:"found"`
	Detected int `json:"detected"`
	Correct  int `json:"correct"`
	Leaked   int `json:"leaked"`
}
type evalMetrics struct {
	Recall    float64    `json:"recall"`
	Precision float64    `json:"precision"`
	LeakRate  float64    `json:"leak_rate"`
	Counts    evalCounts `json:"counts"`
}
type evalResult struct {
	Detector string                 `json:"detector"`
	Errors   int                    `json:"errors"`
	Overall  evalMetrics            `json:"overall"`
	ByKind   map[string]evalMetrics `json:"by_kind"`
}

func metrics(c evalCounts) evalMetrics {
	m := evalMetrics{Counts: c}
	if c.Gold > 0 {
		m.Recall = float64(c.Found) / float64(c.Gold)
		m.LeakRate = float64(c.Leaked) / float64(c.Gold)
	} else {
		m.Recall = 1
	}
	if c.Detected > 0 {
		m.Precision = float64(c.Correct) / float64(c.Detected)
	} else {
		m.Precision = 1
	}
	return m
}

func newShieldEvalCmd(flags *rootFlags) *cobra.Command {
	var samplePath, baseURL, keyEnv, fallback string
	var names []string
	var minRecall float64
	var detectorTimeout time.Duration
	cmd := &cobra.Command{Use: "eval", Short: "Measure detector recall, precision, and masking leaks on JSONL samples", Example: "  mixlayer shield eval --sample testdata/shield-eval/synthetic.jsonl --detector regex --min-recall 0 --agent", RunE: func(cmd *cobra.Command, args []string) error {
		if samplePath == "" {
			return usageErr(fmt.Errorf("--sample is required"))
		}
		if minRecall < 0 || minRecall > 1 {
			return usageErr(fmt.Errorf("--min-recall must be between 0 and 1"))
		}
		samples, err := readEvalSamples(samplePath)
		if err != nil {
			return err
		}
		if len(names) == 0 {
			names = []string{"regex"}
		}
		var results []evalResult
		below := false
		for _, name := range names {
			d := detectorFlags{Name: name, BaseURL: baseURL, KeyEnv: keyEnv, Fallback: fallback, Timeout: detectorTimeout}
			runtime, err := d.runtime(cmd, flags)
			if err != nil {
				return err
			}
			dir, err := os.MkdirTemp("", "mixlayer-shield-eval-")
			if err != nil {
				return err
			}
			s, err := store.Open(filepath.Join(dir, "eval.db"))
			if err != nil {
				os.RemoveAll(dir)
				return err
			}
			result, runErr := evaluateDetector(cmd, s, samples, runtime)
			s.Close()
			os.RemoveAll(dir)
			if runErr != nil {
				return runErr
			}
			if result.Overall.Recall < minRecall {
				below = true
			}
			results = append(results, result)
		}
		if flags.asJSON || flags.agent {
			if err := flags.printJSON(cmd, map[string]any{"results": results, "min_recall": minRecall}); err != nil {
				return err
			}
		} else {
			rows := make([][]string, 0, len(results))
			for _, r := range results {
				rows = append(rows, []string{r.Detector, fmt.Sprintf("%.3f", r.Overall.Recall), fmt.Sprintf("%.3f", r.Overall.Precision), fmt.Sprintf("%.3f", r.Overall.LeakRate), fmt.Sprintf("%d", r.Overall.Counts.Gold), fmt.Sprintf("%d", r.Errors)})
			}
			if err := flags.printTable(cmd, []string{"DETECTOR", "RECALL", "PRECISION", "LEAK_RATE", "GOLD", "ERRORS"}, rows); err != nil {
				return err
			}
		}
		if below {
			return usageErr(fmt.Errorf("detector recall below --min-recall %.3f", minRecall))
		}
		return nil
	}}
	cmd.Flags().StringVar(&samplePath, "sample", "", "JSONL file of synthetic or labeled samples")
	cmd.Flags().StringArrayVar(&names, "detector", nil, "Detector to evaluate (repeatable; default regex)")
	cmd.Flags().StringVar(&baseURL, "detector-base-url", "http://localhost:11434/v1", "OpenAI-compatible local detector base URL")
	cmd.Flags().StringVar(&keyEnv, "detector-key-env", "", "Environment variable name for detector key")
	cmd.Flags().StringVar(&fallback, "detector-fallback", "", "Use regex if model detector fails (regex)")
	cmd.Flags().DurationVar(&detectorTimeout, "detector-timeout", time.Minute, "Maximum time for each detector request")
	cmd.Flags().Float64Var(&minRecall, "min-recall", 0.9, "Exit 2 if any detector's overall recall is below this value")
	return cmd
}

func readEvalSamples(path string) ([]evalSample, error) {
	f, err := os.Open(path)
	if err != nil {
		return nil, err
	}
	defer f.Close()
	scanner := bufio.NewScanner(f)
	scanner.Buffer(make([]byte, 4096), 8<<20)
	var samples []evalSample
	for line := 1; scanner.Scan(); line++ {
		raw := strings.TrimSpace(scanner.Text())
		if raw == "" || strings.HasPrefix(raw, "#") {
			continue
		}
		var sample evalSample
		if err := json.Unmarshal([]byte(raw), &sample); err != nil {
			return nil, fmt.Errorf("sample line %d: %w", line, err)
		}
		if sample.ID == "" {
			return nil, fmt.Errorf("sample line %d: id is required", line)
		}
		samples = append(samples, sample)
	}
	if err := scanner.Err(); err != nil {
		return nil, err
	}
	if len(samples) == 0 {
		return nil, fmt.Errorf("sample contains no records")
	}
	return samples, nil
}

func evaluateDetector(cmd *cobra.Command, s *store.Store, samples []evalSample, d detectorRuntime) (evalResult, error) {
	counts := map[string]evalCounts{}
	errors := 0
	for _, sample := range samples {
		terms, err := s.DictionaryTerms(cmd.Context())
		if err != nil {
			return evalResult{}, err
		}
		entities, usedFallback, err := shield.DetectWith(cmd.Context(), sample.Text, terms, d.Call, d.Fallback)
		if err != nil || usedFallback {
			errors++
			entities = nil
		}
		redacted := sample.Text
		if entities != nil {
			res, err := shield.RedactEntities(cmd.Context(), s, sample.Text, entities, false)
			if err != nil {
				return evalResult{}, err
			}
			redacted = res.Text
		}
		goldSpans := make([]shield.Entity, 0)
		for _, g := range sample.Entities {
			if g.Text == "" {
				continue
			}
			for offset := 0; offset < len(sample.Text); {
				idx := strings.Index(sample.Text[offset:], g.Text)
				if idx < 0 {
					break
				}
				start := offset + idx
				goldSpans = append(goldSpans, shield.Entity{Kind: g.Kind, Start: start, End: start + len(g.Text)})
				offset = start + len(g.Text)
			}
		}
		for _, g := range goldSpans {
			c := counts[g.Kind]
			c.Gold++
			found := false
			for _, e := range entities {
				if e.Start <= g.Start && e.End >= g.End {
					found = true
					break
				}
			}
			if found {
				c.Found++
			}
			if strings.Contains(redacted, sample.Text[g.Start:g.End]) {
				c.Leaked++
			}
			counts[g.Kind] = c
		}
		for _, e := range entities {
			c := counts[e.Kind]
			c.Detected++
			for _, g := range goldSpans {
				if e.Start < g.End && g.Start < e.End {
					c.Correct++
					break
				}
			}
			counts[e.Kind] = c
		}
	}
	all := evalCounts{}
	byKind := map[string]evalMetrics{}
	keys := make([]string, 0, len(counts))
	for k := range counts {
		keys = append(keys, k)
	}
	sort.Strings(keys)
	for _, k := range keys {
		c := counts[k]
		byKind[k] = metrics(c)
		all.Gold += c.Gold
		all.Found += c.Found
		all.Detected += c.Detected
		all.Correct += c.Correct
		all.Leaked += c.Leaked
	}
	return evalResult{Detector: d.Name, Errors: errors, Overall: metrics(all), ByKind: byKind}, nil
}
