// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.
package cli

import (
	"bytes"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	"github.com/mixlayer/cli/internal/systemone"
)

func TestDecideBatchContinuesAndBoundsConcurrency(t *testing.T) {
	var active, peak, calls atomic.Int32
	// Hold the first two workers until both have arrived, proving actual overlap.
	release := make(chan struct{})
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		n := active.Add(1)
		defer active.Add(-1)
		for old := peak.Load(); n > old; old = peak.Load() {
			if peak.CompareAndSwap(old, n) {
				break
			}
		}
		call := calls.Add(1)
		if call == 2 {
			close(release)
		}
		select {
		case <-release:
		case <-r.Context().Done():
			return
		}
		var req systemone.Request
		_ = json.NewDecoder(r.Body).Decode(&req)
		if string(req.State) == `"bad"` {
			w.WriteHeader(422)
			fmt.Fprint(w, `{"error":{"code":"invalid_request","message":"Fixture row rejected"}}`)
			return
		}
		decisionReply(w, "fixture")
	}))
	defer server.Close()
	decisionEnv(t, server.URL)
	input := decisionFile(t, "rows.jsonl", "{\"text\":\"ok\"}\n{\"text\":\"bad\"}\n{\"text\":\"also ok\"}\n{\"text\":\"last\"}\n")
	questions := decisionFile(t, "questions.json", `{"q":{"type":"noul","instructions":"?"}}`)
	flags := decisionFlags()
	cmd := newDecideCmd(flags)
	var stdout, stderr bytes.Buffer
	cmd.SetOut(&stdout)
	cmd.SetErr(&stderr)
	cmd.SilenceErrors = true
	cmd.SilenceUsage = true
	cmd.SetArgs([]string{"batch", "--model", "fixture", "--input", input, "--questions", questions, "--state-field", "text", "--concurrency", "2"})
	err := cmd.Execute()
	if err == nil || ExitCode(err) != 6 {
		t.Fatalf("err=%v", err)
	}
	if peak.Load() != 2 || calls.Load() != 4 {
		t.Fatalf("peak=%d calls=%d", peak.Load(), calls.Load())
	}
	rows := map[int]decisionRowResult{}
	decoder := json.NewDecoder(&stdout)
	for {
		var row decisionRowResult
		err := decoder.Decode(&row)
		if err == io.EOF {
			break
		}
		if err != nil {
			t.Fatal(err)
		}
		rows[row.Row] = row
	}
	if len(rows) != 4 || rows[2].Error == nil || rows[2].Error.Code != 2 || rows[4].Usage.InputTokens != 12 {
		t.Fatalf("rows=%+v", rows)
	}
	if !strings.Contains(stderr.String(), "rows ok=3 failed=1; total input tokens=36") {
		t.Fatal(stderr.String())
	}
}

func TestDecideBatchDryRunMaxRowsAndOutputFile(t *testing.T) {
	var calls atomic.Int32
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { calls.Add(1); decisionReply(w, "fixture") }))
	defer server.Close()
	decisionEnv(t, server.URL)
	input := decisionFile(t, "rows.csv", "text,id\nfirst,1\nsecond,2\nthird,3\n")
	questions := decisionFile(t, "questions.json", `{"q":{"type":"noul","instructions":"?"}}`)
	dest := filepath.Join(t.TempDir(), "results.jsonl")
	flags := decisionFlags()
	flags.dryRun = true
	out, err := runCommand(t, newDecideCmd(flags), "batch", "--model", "fixture", "--input", input, "--questions", questions, "--max-rows", "2", "--out", dest)
	if err != nil || !strings.Contains(out, `"rows": 2`) || !strings.Contains(out, `"text": "first"`) {
		t.Fatalf("out=%s err=%v", out, err)
	}
	if _, err := os.Stat(dest); !os.IsNotExist(err) {
		t.Fatalf("dry run created --out: %v", err)
	}
	if calls.Load() != 0 {
		t.Fatalf("dry-run calls=%d", calls.Load())
	}
	out, err = runCommand(t, newDecideCmd(decisionFlags()), "batch", "--model", "fixture", "--input", input, "--questions", questions, "--max-rows", "2", "--out", dest)
	if err != nil || out != "" || calls.Load() != 2 {
		t.Fatalf("out=%s err=%v calls=%d", out, err, calls.Load())
	}
	data, err := os.ReadFile(dest)
	if err != nil {
		t.Fatal(err)
	}
	if len(bytes.Split(bytes.TrimSpace(data), []byte("\n"))) != 2 {
		t.Fatal(string(data))
	}
}

func TestDecideBatchLocalErrorsContinue(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { decisionReply(w, "fixture") }))
	defer server.Close()
	decisionEnv(t, server.URL)
	questions := decisionFile(t, "questions.json", `{"q":{"type":"noul","instructions":"?"}}`)
	for _, tc := range []struct {
		name, body, field string
		failed            int
	}{
		{"rows.jsonl", "bad\n{\"text\":123}\n{}\n{\"text\":\"ok\"}\n", "text", 3},
		{"rows.csv", "text,id\nwrong\nok,2\n", "", 1},
		{"rows.jsonl", strings.Repeat("x", systemone.MaxBody+1) + "\n{\"text\":\"ok\"}\n", "", 1},
	} {
		input := decisionFile(t, tc.name, tc.body)
		flags := decisionFlags()
		flags.timeout = time.Second
		out, err := runCommand(t, newDecideCmd(flags), "batch", "--model", "fixture", "--input", input, "--questions", questions, "--state-field", tc.field)
		if err == nil || ExitCode(err) != 6 {
			t.Fatalf("err=%v", err)
		}
		if strings.Count(out, `"error":`) != tc.failed || strings.Count(out, `"answers":`) != 1 {
			t.Fatalf("out=%s", out)
		}
	}
}

func TestDecideBatchRejectsUnsafeOutputAndBounds(t *testing.T) {
	decisionEnv(t, "http://127.0.0.1:1")
	input := decisionFile(t, "rows.jsonl", "{}\n")
	questions := decisionFile(t, "questions.json", `{"q":{"type":"noul","instructions":"?"}}`)
	for _, extra := range [][]string{{"--out", input}, {"--out", questions}, {"--concurrency", "0"}, {"--concurrency", "65"}, {"--max-rows", "0"}} {
		args := []string{"batch", "--model", "fixture", "--input", input, "--questions", questions}
		args = append(args, extra...)
		_, err := runCommand(t, newDecideCmd(decisionFlags()), args...)
		if err == nil || ExitCode(err) != 2 {
			t.Fatalf("err=%v", err)
		}
	}
	data, _ := os.ReadFile(input)
	if string(data) != "{}\n" {
		t.Fatal("input overwritten")
	}
}
