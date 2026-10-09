// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

//go:build live

// Live tests call the real Mixlayer API with MIXLAYER_API_KEY. They use free
// models only, unless MIXLAYER_LIVE_PAID=1 allows one short paid call.
//
//	MIXLAYER_API_KEY=... go test -tags live -count=1 -v ./e2e/
package e2e

import (
	"bytes"
	"encoding/json"
	"errors"
	"os"
	"os/exec"
	"path/filepath"
	"sort"
	"strings"
	"testing"
)

// liveRun runs the binary against the real API with an isolated home that is
// shared across one test so the ledger can be inspected afterwards.
func liveRun(t *testing.T, home string, extraEnv []string, args ...string) result {
	t.Helper()
	cmd := exec.Command(binPath, args...)
	cmd.Env = append([]string{
		"HOME=" + home,
		"PATH=" + os.Getenv("PATH"),
		"MIXLAYER_HOME=" + filepath.Join(home, "mixlayer"),
		"MIXLAYER_API_KEY=" + os.Getenv("MIXLAYER_API_KEY"),
		"MIXLAYER_NO_AGENT_DETECT=1",
		"NO_COLOR=1",
	}, extraEnv...)
	for _, name := range []string{"MIXLAYER_BASE_URL", "MIXLAYER_PLATFORM_URL"} {
		if v := os.Getenv(name); v != "" {
			cmd.Env = append(cmd.Env, name+"="+v)
		}
	}
	var out, errb bytes.Buffer
	cmd.Stdout, cmd.Stderr = &out, &errb
	cmd.Stdin = strings.NewReader("")
	err := cmd.Run()
	code := 0
	var exitErr *exec.ExitError
	if errors.As(err, &exitErr) {
		code = exitErr.ExitCode()
	} else if err != nil {
		t.Fatalf("run %v: %v", args, err)
	}
	res := result{stdout: out.String(), stderr: errb.String(), code: code, home: home}
	key := os.Getenv("MIXLAYER_API_KEY")
	if key != "" && (strings.Contains(res.stdout, key) || strings.Contains(res.stderr, key)) {
		t.Fatalf("%v: API key printed", args)
	}
	return res
}

func requireKey(t *testing.T) {
	t.Helper()
	if os.Getenv("MIXLAYER_API_KEY") == "" {
		t.Skip("MIXLAYER_API_KEY not set")
	}
}

type liveModel struct {
	ID     string
	Input  float64
	Output float64
	Priced bool
	Status string
	Type   string
}

// liveCatalog returns the live generation models with prices, cheapest first.
func liveCatalog(t *testing.T, home string) []liveModel {
	t.Helper()
	res := liveRun(t, home, nil, "models", "list", "--live-prices", "--json")
	if res.code != 0 {
		t.Fatalf("models list --live-prices exit %d: %s", res.code, res.stderr)
	}
	var payload struct {
		Models []struct {
			SKU    string   `json:"model_sku"`
			In     *float64 `json:"price_m_input_tokens_usd"`
			Out    *float64 `json:"price_m_output_tokens_usd"`
			Status string   `json:"status"`
			Type   string   `json:"model_type"`
		} `json:"models"`
		Source  string `json:"source"`
		Warning string `json:"warning"`
	}
	if err := json.Unmarshal([]byte(res.stdout), &payload); err != nil {
		t.Fatalf("models list is not JSON: %v\n%s", err, res.stdout)
	}
	t.Logf("catalog source=%s warning=%q", payload.Source, payload.Warning)
	var models []liveModel
	for _, m := range payload.Models {
		lm := liveModel{ID: m.SKU, Status: m.Status, Type: m.Type}
		if m.In != nil && m.Out != nil {
			lm.Input, lm.Output, lm.Priced = *m.In, *m.Out, true
		}
		t.Logf("model %-34s type=%-14s status=%-8s priced=%v in=$%.2f/M out=$%.2f/M", lm.ID, lm.Type, lm.Status, lm.Priced, lm.Input, lm.Output)
		if lm.Type != "" && lm.Type != "generation" && lm.Type != "chat" {
			continue
		}
		models = append(models, lm)
	}
	sort.SliceStable(models, func(i, j int) bool {
		return models[i].Input+models[i].Output < models[j].Input+models[j].Output
	})
	return models
}

func freeModel(t *testing.T, models []liveModel) string {
	t.Helper()
	for _, m := range models {
		if m.Priced && m.Input == 0 && m.Output == 0 {
			return m.ID
		}
	}
	for _, m := range models {
		if strings.Contains(m.ID, "free") {
			return m.ID
		}
	}
	t.Skip("no free model in the live catalog")
	return ""
}

func TestLiveDoctor(t *testing.T) {
	requireKey(t)
	res := liveRun(t, t.TempDir(), nil, "doctor", "--json")
	requireJSON(t, "doctor", res.stdout)
	if res.code != 0 {
		t.Fatalf("doctor exit %d: %s\n%s", res.code, res.stdout, res.stderr)
	}
}

func TestLiveFreeModelRoundTrip(t *testing.T) {
	requireKey(t)
	home := t.TempDir()
	model := freeModel(t, liveCatalog(t, home))
	t.Logf("free model: %s", model)

	res := liveRun(t, home, nil, "ask", "Reply with exactly the word: pong", "--model", model, "--agent")
	if res.code != 0 {
		t.Fatalf("ask exit %d: %s", res.code, res.stderr)
	}
	if !strings.Contains(strings.ToLower(res.stdout), "pong") {
		t.Errorf("ask answer missing pong: %s", res.stdout)
	}

	// Masked round trip on synthetic data: the stored outbound prompt must not
	// contain the raw identifiers.
	secret := []string{"jordan.avery@example.com", "555-013-4477"}
	q := "Write one sentence asking " + secret[0] + " to call " + secret[1] + " about invoice 12."
	res = liveRun(t, home, nil, "shield", "ask", q, "--model", model, "--agent")
	if res.code != 0 {
		t.Fatalf("shield ask exit %d: %s\n%s", res.code, res.stdout, res.stderr)
	}
	res = liveRun(t, home, nil, "sql", "select prompt from runs where command like '%shield%' order by created_at desc limit 1", "--json")
	if res.code != 0 {
		t.Fatalf("sql exit %d: %s", res.code, res.stderr)
	}
	if !strings.Contains(res.stdout, "prompt") {
		t.Fatalf("no shield run recorded: %s", res.stdout)
	}
	for _, s := range secret {
		if strings.Contains(res.stdout, s) {
			t.Errorf("stored outbound prompt contains raw %q", s)
		} else {
			t.Logf("outbound prompt: %q absent", s)
		}
	}

	// Multi-model commands restricted to the free model.
	for _, args := range [][]string{
		{"ladder", "What is 2+2? Answer with the number only.", "--rungs", model, "--agent"},
		{"escalate", "What is 2+2? Answer with the number only.", "--rungs", model, "--agent"},
	} {
		res = liveRun(t, home, nil, args...)
		if res.code != 0 {
			t.Errorf("%s exit %d: %s", args[0], res.code, res.stderr)
		}
	}
}

func TestLiveOnePaidCall(t *testing.T) {
	requireKey(t)
	if os.Getenv("MIXLAYER_LIVE_PAID") != "1" {
		t.Skip("set MIXLAYER_LIVE_PAID=1 to allow one short paid call")
	}
	home := t.TempDir()
	var paid string
	for _, m := range liveCatalog(t, home) {
		if m.Priced && m.Input+m.Output > 0 {
			paid = m.ID
			break
		}
	}
	if paid == "" {
		t.Skip("no priced paid model in the live catalog")
	}
	res := liveRun(t, home, nil, "ask", "Reply with exactly the word: pong", "--model", paid, "--agent")
	switch res.code {
	case 0:
		t.Logf("paid model %s answered", paid)
	case 8:
		t.Skipf("paid model %s: no prepaid credit (exit 8, as documented)", paid)
	default:
		t.Fatalf("paid ask on %s exit %d: %s", paid, res.code, res.stderr)
	}
}

func TestLiveJiniDecision(t *testing.T) {
	requireKey(t)
	model := os.Getenv("MIXLAYER_LIVE_JINI_MODEL")
	if model == "" {
		t.Skip("set MIXLAYER_LIVE_JINI_MODEL to test the experimental decision endpoint")
	}
	env := []string{"MIXLAYER_ENABLE_JINI=1"}
	res := liveRun(t, t.TempDir(), env, "decide", "--model", model,
		"--state-text", "The customer wrote: my card was charged twice for order 1182.",
		"--noul", "billing=Is this a billing problem?", "--agent")
	if res.code != 0 {
		t.Fatalf("decide exit %d: %s\n%s", res.code, res.stdout, res.stderr)
	}
	t.Logf("decide: %s", strings.TrimSpace(res.stdout))
}
