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

func decisionEnv(t *testing.T, base string) {
	t.Helper()
	t.Setenv("MIXLAYER_API_KEY", "")
	t.Setenv("MIXLAYER_CONFIG", filepath.Join(t.TempDir(), "config.toml"))
	t.Setenv("MIXLAYER_CONFIG_DIR", t.TempDir())
	t.Setenv("MIXLAYER_DATA_DIR", t.TempDir())
	t.Setenv("MIXLAYER_BASE_URL", base)
	t.Setenv("MIXLAYER_PLATFORM_URL", base)
}
func decisionFile(t *testing.T, name, body string) string {
	t.Helper()
	p := filepath.Join(t.TempDir(), name)
	if err := os.WriteFile(p, []byte(body), 0600); err != nil {
		t.Fatal(err)
	}
	return p
}
func decisionReply(w http.ResponseWriter, model string) {
	w.Header().Set("Content-Type", "application/json")
	fmt.Fprintf(w, `{"model":%q,"answers":{"q":{"type":"noul","noul":0.8}},"usage":{"input_tokens":12,"output_tokens":0}}`, model)
}
func decisionFlags() *rootFlags { return &rootFlags{asJSON: true, timeout: 3 * time.Second} }

func TestDecideRequestGolden(t *testing.T) {
	var body []byte
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Method != "POST" || r.URL.Path != "/v1/systemone" {
			t.Errorf("request = %s %s", r.Method, r.URL.Path)
		}
		body, _ = io.ReadAll(r.Body)
		fmt.Fprint(w, `{"model":"fixture","answers":{"dup":{"type":"noul","noul":0.8},"route":{"type":"choice","choice":"review","probabilities":{"review":0.9,"ignore":0.1},"confidence":0.8}},"usage":{"input_tokens":25,"output_tokens":0}}`)
	}))
	defer server.Close()
	decisionEnv(t, server.URL+"/v1")
	questions := decisionFile(t, "questions.json", `{"route":{"type":"choice","instructions":"Route?","criteria":{"review":"Review","ignore":"Ignore"}}}`)
	out, err := runCommand(t, newDecideCmd(decisionFlags()), "--model", "fixture", "--state-text", "Two charges", "--noul", "dup=Duplicate?", "--questions", questions)
	if err != nil {
		t.Fatal(err)
	}
	golden, err := os.ReadFile("../../testdata/decide-request.golden.json")
	if err != nil {
		t.Fatal(err)
	}
	if string(body) != strings.TrimSpace(string(golden)) {
		t.Fatalf("request = %s\nwant = %s", body, golden)
	}
	if !strings.Contains(out, `"input_tokens": 25`) {
		t.Fatal(out)
	}
}

func TestDecideStateDetection(t *testing.T) {
	for _, tc := range []struct{ input, want string }{
		{`{"large":9007199254740993}`, `{"large":9007199254740993}`}, {`[1,"two"]`, `[1,"two"]`},
		{"plain text", `"plain text"`}, {`123`, `"123"`}, {`true`, `"true"`}, {`null`, `"null"`}, {`"quoted"`, `"\"quoted\""`}, {`{bad`, `"{bad"`},
	} {
		t.Run(tc.input, func(t *testing.T) {
			if got := string(systemone.State([]byte(tc.input))); got != tc.want {
				t.Fatalf("got %s want %s", got, tc.want)
			}
		})
	}
}

func TestDecideFailIf(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		fmt.Fprint(w, `{"model":"fixture","answers":{"q":{"type":"noul","noul":0.8},"s":{"type":"score","score":0.8,"legend":{"0":"low","1":"high"},"probabilities":{"0":0.2,"1":0.8},"confidence":0.6},"c":{"type":"choice","choice":"review","probabilities":{"review":0.9,"ignore":0.1},"confidence":0.8}},"usage":{"input_tokens":25,"output_tokens":0}}`)
	}))
	defer server.Close()
	decisionEnv(t, server.URL)
	q := `{"q":{"type":"noul","instructions":"?"},"s":{"type":"score","instructions":"Rate?","criteria":["low","high"]},"c":{"type":"choice","instructions":"Pick?","criteria":{"review":"Review","ignore":"Ignore"}}}`
	for _, tc := range []struct {
		expr    string
		matches bool
	}{
		{"q>0.7", true}, {"q>0.8", false}, {"q>=0.8", true}, {"q>=0.9", false}, {"q<0.9", true}, {"q<0.8", false}, {"q<=0.8", true}, {"q<=0.7", false},
		{"s>0.7", true}, {"s>=0.8", true}, {"s<0.9", true}, {"s<=0.8", true}, {"c==review", true}, {"c==ignore", false},
	} {
		t.Run(tc.expr, func(t *testing.T) {
			out, err := runCommand(t, newDecideCmd(decisionFlags()), "--model", "fixture", "--state-text", "facts", "--questions-json", q, "--fail-if", tc.expr)
			if tc.matches {
				if err == nil || ExitCode(err) != 9 {
					t.Fatalf("error=%v", err)
				}
			} else if err != nil {
				t.Fatal(err)
			}
			var res systemone.Response
			if err := json.Unmarshal([]byte(out), &res); err != nil || len(res.Answers) != 3 {
				t.Fatalf("output=%s", out)
			}
		})
	}
}

func TestDecideDryRunAndValidationNeverCallHTTP(t *testing.T) {
	var calls atomic.Int32
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { calls.Add(1); decisionReply(w, "fixture") }))
	defer server.Close()
	decisionEnv(t, server.URL)
	flags := decisionFlags()
	flags.dryRun = true
	cmd := newDecideCmd(flags)
	cmd.SetIn(strings.NewReader(`{"nested":[1,2]}`))
	out, err := runCommand(t, cmd, "--model", "fixture", "--state", "-", "--noul", "q=Valid?")
	if err != nil || !strings.Contains(out, `"url": "`+server.URL+`/v1/systemone"`) || !strings.Contains(out, `"nested"`) {
		t.Fatalf("out=%s err=%v", out, err)
	}
	for _, args := range [][]string{
		{"--state-text", "facts", "--noul", "q=?"},
		{"--model", "fixture", "--state-text", "facts", "--noul", "q=?", "--noul", "q=again"},
		{"--model", "fixture", "--state-text", "facts", "--questions-json", `{"q":{"type":"noul","instructions":"?"},"q":{"type":"noul","instructions":"?"}}`},
		{"--model", "fixture", "--state-text", "facts", "--noul", "q=?", "--fail-if", "missing>1"},
		{"--model", "fixture", "--state-text", "facts", "--noul", "q=?", "--fail-if", "q>=NaN"},
		{"--model", "fixture", "--state-text", "facts", "--noul", "q=?", "--fail-if", "q==yes"},
		{"--model", "fixture", "--state-text", "facts", "--noul", " =?"},
		{"--model", "fixture", "--state-text", "facts", "--noul", "q=" + strings.Repeat("x", systemone.MaxQuestion+1)},
		{"--model", "fixture", "--state-text", strings.Repeat("x", systemone.MaxState+1), "--noul", "q=?"},
		{"--model", "fixture", "--state", "-", "--questions", "-"},
	} {
		_, err := runCommand(t, newDecideCmd(decisionFlags()), args...)
		if err == nil || ExitCode(err) != 2 {
			t.Fatalf("error=%v", err)
		}
	}
	if calls.Load() != 0 {
		t.Fatalf("HTTP calls=%d", calls.Load())
	}
}

func TestDecideConfigModelAndEndpoint(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { decisionReply(w, "configured") }))
	defer server.Close()
	decisionEnv(t, server.URL)
	cfg := decisionFile(t, "config.toml", "classification_model = \"configured\"\n")
	flags := decisionFlags()
	flags.configPath = cfg
	out, err := runCommand(t, newDecideCmd(flags), "--endpoint", server.URL+"/v1/systemone", "--state-text", "facts", "--noul", "q=?")
	if err != nil || !strings.Contains(out, "configured") {
		t.Fatalf("out=%s err=%v", out, err)
	}
}

func TestDecideStatusRetryAndHints(t *testing.T) {
	for _, tc := range []struct {
		status, code, calls int
		success             bool
	}{
		{401, 4, 1, false}, {402, 8, 1, false}, {403, 4, 1, false}, {422, 2, 1, false}, {429, 7, 4, false}, {529, 5, 4, false}, {500, 5, 1, false}, {503, 5, 1, false}, {529, 0, 2, true}, {429, 0, 2, true},
	} {
		t.Run(fmt.Sprint(tc), func(t *testing.T) {
			var calls atomic.Int32
			server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				n := calls.Add(1)
				if tc.success && n == 2 {
					decisionReply(w, "fixture")
					return
				}
				w.Header().Set("Retry-After", "0")
				w.WriteHeader(tc.status)
				fmt.Fprint(w, `{"error":{"code":"fixture_error","message":"Fixture rejection"}}`)
			}))
			defer server.Close()
			decisionEnv(t, server.URL)
			_, err := runCommand(t, newDecideCmd(decisionFlags()), "--model", "fixture", "--state-text", "facts", "--noul", "q=?")
			if tc.success {
				if err != nil {
					t.Fatal(err)
				}
			} else if err == nil || ExitCode(err) != tc.code || !strings.Contains(err.Error(), "hint:") {
				t.Fatalf("error=%v", err)
			}
			if int(calls.Load()) != tc.calls {
				t.Fatalf("calls=%d want %d", calls.Load(), tc.calls)
			}
		})
	}
}

func TestDecideMalformedResponseFailsClosed(t *testing.T) {
	for _, body := range []string{`{}`, `{"model":"fixture","answers":{"q":{"type":"noul"}},"usage":{"input_tokens":1,"output_tokens":0}}`, `{"model":"fixture","answers":{"q":{"type":"noul","noul":1.1}},"usage":{"input_tokens":1,"output_tokens":0}}`} {
		server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { fmt.Fprint(w, body) }))
		decisionEnv(t, server.URL)
		_, err := runCommand(t, newDecideCmd(decisionFlags()), "--model", "fixture", "--state-text", "facts", "--noul", "q=?", "--fail-if", "q>0.5")
		server.Close()
		if err == nil || ExitCode(err) != 5 {
			t.Fatalf("error=%v", err)
		}
	}
}

func TestDecideHumanOutput(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { decisionReply(w, "fixture") }))
	defer server.Close()
	decisionEnv(t, server.URL)
	flags := decisionFlags()
	flags.asJSON = false
	out, err := runCommand(t, newDecideCmd(flags), "--model", "fixture", "--state-text", "facts", "--noul", "q=?")
	if err != nil || !strings.Contains(out, "CONFIDENCE") || !strings.Contains(out, "0.8") {
		t.Fatalf("out=%s err=%v", out, err)
	}
}

func TestDecideStateTextRemainsLiteral(t *testing.T) {
	decisionEnv(t, "http://127.0.0.1:1")
	flags := decisionFlags()
	flags.dryRun = true
	cmd := newDecideCmd(flags)
	var out bytes.Buffer
	cmd.SetOut(&out)
	cmd.SetArgs([]string{"--model", "fixture", "--state-text", `{"literal":true}`, "--noul", "q=?"})
	if err := cmd.Execute(); err != nil {
		t.Fatal(err)
	}
	var preview struct{ Body systemone.Request }
	if err := json.Unmarshal(out.Bytes(), &preview); err != nil {
		t.Fatal(err)
	}
	if string(preview.Body.State) != `"{\"literal\":true}"` {
		t.Fatal(string(preview.Body.State))
	}
}

func TestDecideAgentPreviewPreservesLargeIntegers(t *testing.T) {
	decisionEnv(t, "http://127.0.0.1:1")
	state := decisionFile(t, "state.json", `{"order_id":9007199254740993}`)
	out, err := runCommand(t, RootCmd(), "decide", "--dry-run", "--agent", "--model", "fixture", "--state", state, "--noul", "q=?")
	if err != nil || !strings.Contains(out, "9007199254740993") || strings.Contains(out, "9007199254740992") {
		t.Fatalf("out=%s err=%v", out, err)
	}
}
