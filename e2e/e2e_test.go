// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

// Package e2e runs the built mixlayer binary as a black box against a local
// fake Mixlayer API. It needs no network and no real key.
package e2e

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"io/fs"
	"net/http"
	"net/http/httptest"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"sync/atomic"
	"testing"
)

const dummyKey = "mlk-e2e-dummy-key-0f9e8d7c6b5a"

var binPath string

func TestMain(m *testing.M) {
	dir, err := os.MkdirTemp("", "mixlayer-e2e-bin")
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
	name := "mixlayer"
	if runtime.GOOS == "windows" {
		name += ".exe"
	}
	binPath = filepath.Join(dir, name)
	build := exec.Command("go", "build", "-o", binPath, "../cmd/mixlayer")
	if out, err := build.CombinedOutput(); err != nil {
		fmt.Fprintf(os.Stderr, "build failed: %v\n%s", err, out)
		os.Exit(1)
	}
	code := m.Run()
	_ = os.RemoveAll(dir)
	os.Exit(code)
}

// fakeAPI is a minimal stand-in for the Mixlayer API. status forces every
// request to fail with that HTTP status when non-zero.
type fakeAPI struct {
	hits   atomic.Int32
	status int
}

func (f *fakeAPI) ServeHTTP(w http.ResponseWriter, r *http.Request) {
	f.hits.Add(1)
	w.Header().Set("Content-Type", "application/json")
	if f.status != 0 {
		if f.status == http.StatusTooManyRequests {
			w.Header().Set("Retry-After", "0")
		}
		w.WriteHeader(f.status)
		code := map[int]string{
			401: "invalid_api_key", 402: "insufficient_prepaid_credits", 404: "model_not_found",
			429: "rate_limited", 500: "internal_error",
		}[f.status]
		fmt.Fprintf(w, `{"error":{"message":"fake %d","code":%q}}`, f.status, code)
		return
	}
	if got := r.Header.Get("Authorization"); got != "Bearer "+dummyKey {
		w.WriteHeader(http.StatusUnauthorized)
		fmt.Fprint(w, `{"error":{"message":"missing key","code":"invalid_api_key"}}`)
		return
	}
	switch {
	case strings.HasSuffix(r.URL.Path, "/models"):
		fmt.Fprint(w, `{"object":"list","data":[{"id":"e2e/free-model","object":"model"},{"id":"e2e/paid-model","object":"model"}]}`)
	case strings.HasSuffix(r.URL.Path, "/chat/completions"):
		fmt.Fprint(w, `{"id":"chatcmpl-e2e","object":"chat.completion","model":"e2e/free-model","choices":[{"index":0,"message":{"role":"assistant","content":"e2e answer 42"},"finish_reason":"stop"}],"usage":{"prompt_tokens":5,"completion_tokens":3,"total_tokens":8}}`)
	default:
		w.WriteHeader(http.StatusNotFound)
		fmt.Fprint(w, `{"error":{"message":"not found","code":"not_found"}}`)
	}
}

type result struct {
	stdout, stderr string
	code           int
	home           string
}

// run executes the binary with an isolated home and environment pointing at
// srv (or an unroutable address when srv is nil).
func run(t *testing.T, srv *httptest.Server, args ...string) result {
	t.Helper()
	home := t.TempDir()
	base := "http://127.0.0.1:9/v1"
	if srv != nil {
		base = srv.URL + "/v1"
	}
	cmd := exec.Command(binPath, args...)
	cmd.Env = []string{
		"HOME=" + home,
		"PATH=" + os.Getenv("PATH"),
		"MIXLAYER_HOME=" + filepath.Join(home, "mixlayer"),
		"MIXLAYER_API_KEY=" + dummyKey,
		"MIXLAYER_BASE_URL=" + base,
		"MIXLAYER_PLATFORM_URL=" + base,
		"MIXLAYER_NO_AGENT_DETECT=1",
		"NO_COLOR=1",
	}
	if runtime.GOOS == "windows" {
		cmd.Env = append(cmd.Env, "USERPROFILE="+home, "APPDATA="+filepath.Join(home, "AppData"), "LOCALAPPDATA="+filepath.Join(home, "Local"))
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
	assertNoKeyLeak(t, res, args)
	return res
}

// assertNoKeyLeak fails if the dummy key appears in output or any file the
// CLI wrote under its isolated home.
func assertNoKeyLeak(t *testing.T, res result, args []string) {
	t.Helper()
	if strings.Contains(res.stdout, dummyKey) || strings.Contains(res.stderr, dummyKey) {
		t.Errorf("%v: API key printed to stdout/stderr", args)
	}
	_ = filepath.WalkDir(res.home, func(path string, d fs.DirEntry, err error) error {
		if err != nil || d.IsDir() {
			return nil
		}
		b, readErr := os.ReadFile(path)
		if readErr == nil && bytes.Contains(b, []byte(dummyKey)) {
			t.Errorf("%v: API key written to %s", args, path)
		}
		return nil
	})
}

func requireJSON(t *testing.T, label, s string) any {
	t.Helper()
	var v any
	if err := json.Unmarshal([]byte(strings.TrimSpace(s)), &v); err != nil {
		t.Fatalf("%s: stdout is not JSON: %v\n%s", label, err, s)
	}
	return v
}

func TestVersion(t *testing.T) {
	res := run(t, nil, "--version")
	if res.code != 0 || !strings.HasPrefix(res.stdout, "mixlayer ") {
		t.Fatalf("--version: code=%d out=%q", res.code, res.stdout)
	}
}

func TestHelpHasNoGeneratorOrExperimentalLeaks(t *testing.T) {
	res := run(t, nil, "--help")
	if res.code != 0 {
		t.Fatalf("--help exit %d", res.code)
	}
	low := strings.ToLower(res.stdout)
	for _, banned := range []string{"printing", "pp-cli", "decide", "jini"} {
		if strings.Contains(low, banned) {
			t.Errorf("--help mentions %q", banned)
		}
	}
}

func TestAgentContextIsJSON(t *testing.T) {
	res := run(t, nil, "agent-context")
	if res.code != 0 {
		t.Fatalf("agent-context exit %d: %s", res.code, res.stderr)
	}
	requireJSON(t, "agent-context", res.stdout)
}

func TestDoctorJSON(t *testing.T) {
	srv := httptest.NewServer(&fakeAPI{})
	defer srv.Close()
	res := run(t, srv, "doctor", "--json")
	v := requireJSON(t, "doctor", res.stdout)
	if _, ok := v.(map[string]any); !ok {
		t.Fatalf("doctor JSON is not an object: %s", res.stdout)
	}
}

func TestModelsListAgent(t *testing.T) {
	srv := httptest.NewServer(&fakeAPI{})
	defer srv.Close()
	res := run(t, srv, "models", "list", "--agent")
	if res.code != 0 {
		t.Fatalf("models list exit %d: %s", res.code, res.stderr)
	}
	requireJSON(t, "models list", res.stdout)
	if !strings.Contains(res.stdout, "e2e/free-model") {
		t.Fatalf("models list missing fake model: %s", res.stdout)
	}
}

func TestAskReturnsAnswer(t *testing.T) {
	srv := httptest.NewServer(&fakeAPI{})
	defer srv.Close()
	res := run(t, srv, "ask", "What is six times seven?", "--model", "e2e/free-model", "--agent")
	if res.code != 0 {
		t.Fatalf("ask exit %d: %s", res.code, res.stderr)
	}
	if !strings.Contains(res.stdout, "e2e answer 42") {
		t.Fatalf("ask output missing answer: %s", res.stdout)
	}
}

func TestDryRunMakesNoRequests(t *testing.T) {
	fake := &fakeAPI{}
	srv := httptest.NewServer(fake)
	defer srv.Close()
	for _, args := range [][]string{
		{"models", "list", "--dry-run", "--agent"},
		{"ask", "hello", "--model", "e2e/free-model", "--dry-run", "--agent"},
		{"shield", "ask", "Email jordan@example.com about the invoice", "--model", "e2e/free-model", "--dry-run", "--agent"},
	} {
		res := run(t, srv, args...)
		if res.code != 0 {
			t.Errorf("%v: exit %d: %s", args, res.code, res.stderr)
		}
	}
	if n := fake.hits.Load(); n != 0 {
		t.Fatalf("--dry-run sent %d requests", n)
	}
}

func TestShieldDryRunMasksEmail(t *testing.T) {
	res := run(t, nil, "shield", "ask", "Email jordan@example.com about the invoice", "--model", "e2e/free-model", "--dry-run", "--agent")
	if res.code != 0 {
		t.Fatalf("shield ask --dry-run exit %d: %s", res.code, res.stderr)
	}
	if strings.Contains(res.stdout, "jordan@example.com") {
		t.Fatalf("dry-run preview contains the raw email: %s", res.stdout)
	}
}

func TestExitCodes(t *testing.T) {
	cases := []struct {
		status, want int
	}{
		{http.StatusUnauthorized, 4},
		{http.StatusPaymentRequired, 8},
		{http.StatusNotFound, 3},
		{http.StatusTooManyRequests, 7},
		{http.StatusInternalServerError, 5},
	}
	for _, tc := range cases {
		t.Run(fmt.Sprint(tc.status), func(t *testing.T) {
			fake := &fakeAPI{status: tc.status}
			srv := httptest.NewServer(fake)
			defer srv.Close()
			res := run(t, srv, "ask", "hello", "--model", "e2e/free-model", "--agent")
			if res.code != tc.want {
				t.Fatalf("HTTP %d: exit %d, want %d\nstderr: %s", tc.status, res.code, tc.want, res.stderr)
			}
			if tc.status == http.StatusPaymentRequired && fake.hits.Load() != 1 {
				t.Fatalf("HTTP 402 was retried: %d requests", fake.hits.Load())
			}
		})
	}
}

func TestUsageErrorExitsTwo(t *testing.T) {
	res := run(t, nil, "ask", "--no-such-flag")
	if res.code != 2 {
		t.Fatalf("unknown flag: exit %d, want 2", res.code)
	}
}
