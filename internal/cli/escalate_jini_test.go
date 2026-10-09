// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.
package cli

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"path/filepath"
	"strings"
	"sync/atomic"
	"testing"

	"github.com/mixlayer/cli/internal/ladder"
	"github.com/mixlayer/cli/internal/store"
	"github.com/mixlayer/cli/internal/systemone"
)

func TestEscalateJiniStopsAtRungTwoAndPersistsChecker(t *testing.T) {
	var chats, checks atomic.Int32
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/v1/chat/completions":
			n := chats.Add(1)
			fmt.Fprintf(w, `{"id":"fixture","model":"rung%d","choices":[{"message":{"role":"assistant","content":"Answer %d. CONFIDENCE: high"}}],"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15}}`, n, n)
		case "/v1/systemone":
			n := checks.Add(1)
			var req systemone.Request
			if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
				t.Error(err)
			}
			if req.Model != "checker-fixture" || req.Questions["correct"].Type != "noul" {
				t.Errorf("bad checker request: %+v", req)
			}
			var state map[string]string
			_ = json.Unmarshal(req.State, &state)
			if state["request"] != "Explain this" || !strings.Contains(state["response"], fmt.Sprintf("Answer %d", n)) {
				t.Errorf("state=%v", state)
			}
			var criteria map[string]string
			_ = json.Unmarshal(req.Questions["correct"].Criteria, &criteria)
			if criteria["true"] == "" || criteria["false"] == "" {
				t.Error("missing criteria")
			}
			score := 0.2
			if n == 2 {
				score = 0.8
			}
			fmt.Fprintf(w, `{"model":"checker-fixture","answers":{"correct":{"type":"noul","noul":%g}},"usage":{"input_tokens":20,"output_tokens":0}}`, score)
		default:
			w.WriteHeader(404)
			fmt.Fprint(w, `{"error":"fixture catalog unavailable"}`)
		}
	}))
	defer server.Close()
	decisionEnv(t, server.URL+"/v1")
	db := filepath.Join(t.TempDir(), "ledger.db")
	out, err := runCommand(t, newEscalateCmd(decisionFlags()), "Explain this", "--rungs", "rung1,rung2,rung3", "--checker", "jini", "--checker-model", "checker-fixture", "--db", db)
	if err != nil {
		t.Fatal(err)
	}
	if chats.Load() != 2 || checks.Load() != 2 {
		t.Fatalf("chats=%d checks=%d", chats.Load(), checks.Load())
	}
	var result struct {
		Results []ladder.Result
		Cost    *float64 `json:"cost_usd"`
		Saved   *float64 `json:"saved_usd"`
	}
	if err := json.Unmarshal([]byte(out), &result); err != nil {
		t.Fatal(err)
	}
	if len(result.Results) != 2 || result.Results[1].Checker != "jini" || result.Results[1].CheckerModel != "checker-fixture" || *result.Results[1].CheckerScore != 0.8 {
		t.Fatal(out)
	}
	if result.Cost != nil || result.Saved != nil {
		t.Fatal("costs must not silently omit checker costs")
	}
	s, err := store.OpenWithContext(context.Background(), db)
	if err != nil {
		t.Fatal(err)
	}
	defer s.Close()
	runs, err := s.RecentRuns(context.Background(), 10)
	if err != nil || len(runs) != 2 {
		t.Fatalf("runs=%d err=%v", len(runs), err)
	}
	for _, run := range runs {
		if !strings.Contains(string(run.RawJSON), `"checker":"jini"`) || !strings.Contains(string(run.RawJSON), `"checker_score":`) || !strings.Contains(string(run.RawJSON), `"checker_model":"checker-fixture"`) {
			t.Fatalf("raw=%s", run.RawJSON)
		}
	}
}

func TestEscalateJiniErrorStopsWithoutHeuristicFallback(t *testing.T) {
	for _, badResponse := range []bool{false, true} {
		t.Run(fmt.Sprint(badResponse), func(t *testing.T) {
			var chats, checks atomic.Int32
			server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				switch r.URL.Path {
				case "/v1/chat/completions":
					chats.Add(1)
					fmt.Fprint(w, `{"model":"rung1","choices":[{"message":{"role":"assistant","content":"Answer. CONFIDENCE: high"}}],"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15}}`)
				case "/v1/systemone":
					checks.Add(1)
					if badResponse {
						fmt.Fprint(w, `{"model":"checker-fixture","answers":{},"usage":{"input_tokens":1,"output_tokens":0}}`)
					} else {
						w.WriteHeader(422)
						fmt.Fprint(w, `{"error":{"code":"invalid_request","message":"Unknown model"}}`)
					}
				default:
					w.WriteHeader(404)
				}
			}))
			defer server.Close()
			decisionEnv(t, server.URL+"/v1")
			out, err := runCommand(t, newEscalateCmd(decisionFlags()), "Explain this", "--rungs", "rung1,rung2,rung3", "--checker", "jini", "--checker-model", "checker-fixture", "--db", filepath.Join(t.TempDir(), "ledger.db"))
			want := 2
			if badResponse {
				want = 5
			}
			if err == nil || ExitCode(err) != want || !strings.Contains(err.Error(), "jini checker failed after rung rung1") {
				t.Fatalf("err=%v", err)
			}
			if chats.Load() != 1 || checks.Load() != 1 || !strings.Contains(out, `"checker_error"`) || !strings.Contains(out, `"checker_score": null`) {
				t.Fatalf("out=%s chats=%d checks=%d", out, chats.Load(), checks.Load())
			}
		})
	}
}

func TestEscalateJiniDryRunAndFlagValidation(t *testing.T) {
	var calls atomic.Int32
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { calls.Add(1) }))
	defer server.Close()
	decisionEnv(t, server.URL+"/v1")
	for _, args := range [][]string{
		{"Hello", "--checker", "jini", "--checker-model", "fixture"},
		{"Hello", "--checker", "wrong"},
		{"Hello", "--checker", "jini"},
		{"Hello", "--checker", "jini", "--checker-model", "fixture", "--threshold", "NaN"},
		{"Hello", "--checker", "jini", "--checker-model", "fixture", "--threshold", "1.1"},
	} {
		flags := decisionFlags()
		flags.dryRun = true
		out, err := runCommand(t, newEscalateCmd(flags), args...)
		if len(args) == 5 {
			if err != nil || !strings.Contains(out, "checker_body_template") {
				t.Fatalf("out=%s err=%v", out, err)
			}
		} else if err == nil || ExitCode(err) != 2 {
			t.Fatalf("err=%v", err)
		}
	}
	if calls.Load() != 0 {
		t.Fatalf("dry run made %d calls", calls.Load())
	}
}
