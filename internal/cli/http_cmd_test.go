package cli

import (
	"bytes"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/spf13/cobra"
)

func runCommand(t *testing.T, cmd *cobra.Command, args ...string) (string, error) {
	t.Helper()
	var out, stderr bytes.Buffer
	cmd.SetOut(&out)
	cmd.SetErr(&stderr)
	cmd.SilenceUsage = true
	cmd.SilenceErrors = true
	cmd.SetArgs(args)
	err := cmd.Execute()
	return out.String(), err
}

func TestAskHTTP402ExitsBilling(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusPaymentRequired)
		_, _ = w.Write([]byte(`{"error":"insufficient_prepaid_credits"}`))
	}))
	defer server.Close()
	t.Setenv("MIXLAYER_API_KEY", "dummy")
	t.Setenv("MIXLAYER_BASE_URL", server.URL+"/v1")
	t.Setenv("MIXLAYER_PLATFORM_URL", server.URL)
	t.Setenv("MIXLAYER_DATA_DIR", t.TempDir())
	t.Setenv("MIXLAYER_CONFIG_DIR", t.TempDir())
	flags := &rootFlags{timeout: 3 * time.Second}
	_, err := runCommand(t, newAskCmd(flags), "Hello", "--db", filepath.Join(t.TempDir(), "data.db"))
	if err == nil || ExitCode(err) != 8 || !strings.Contains(err.Error(), "console.mixlayer.com/app/billing") {
		t.Fatalf("err=%v", err)
	}
}

func TestShieldAskHTTP402ExitsBilling(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusPaymentRequired)
		_, _ = w.Write([]byte(`{"error":"insufficient_prepaid_credits"}`))
	}))
	defer server.Close()
	t.Setenv("TEST_FRONTIER_KEY", "dummy")
	_, err := executeShieldAsk(t, "Hello", "--upstream", "custom", "--accept-regex-only", "--upstream-base-url", server.URL+"/v1", "--upstream-key-env", "TEST_FRONTIER_KEY", "--model", "fake", "--db", filepath.Join(t.TempDir(), "data.db"))
	if err == nil || ExitCode(err) != 8 {
		t.Fatalf("err=%v", err)
	}
}

func TestLadderAllFailExitsNonZero(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusNotFound)
		_, _ = w.Write([]byte(`{"error":"model not found"}`))
	}))
	defer server.Close()
	t.Setenv("MIXLAYER_API_KEY", "dummy")
	t.Setenv("MIXLAYER_BASE_URL", server.URL+"/v1")
	t.Setenv("MIXLAYER_PLATFORM_URL", server.URL)
	t.Setenv("MIXLAYER_DATA_DIR", t.TempDir())
	t.Setenv("MIXLAYER_CONFIG_DIR", t.TempDir())
	flags := &rootFlags{timeout: 3 * time.Second}
	out, err := runCommand(t, newLadderCmd(flags), "Hello", "--rungs", "fake", "--db", filepath.Join(t.TempDir(), "data.db"))
	if err == nil || ExitCode(err) != 3 || !strings.Contains(out, "model not found") {
		t.Fatalf("err=%v output=%s", err, out)
	}
}

func TestChatMalformed200ExitsAPIError(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		_, _ = w.Write([]byte("<html>broken</html>"))
	}))
	defer server.Close()
	t.Setenv("MIXLAYER_API_KEY", "dummy")
	t.Setenv("MIXLAYER_BASE_URL", server.URL+"/v1")
	t.Setenv("MIXLAYER_DATA_DIR", t.TempDir())
	t.Setenv("MIXLAYER_CONFIG_DIR", t.TempDir())
	flags := &rootFlags{timeout: 3 * time.Second}
	_, err := runCommand(t, newChatPromotedCmd(flags), "--model", "fake", "--messages", `[{"role":"user","content":"hi"}]`)
	if err == nil || ExitCode(err) != 5 {
		t.Fatalf("err=%v", err)
	}
}

func TestLadderPartialFailureExitsSix(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/v1/chat/completions" {
			w.WriteHeader(404)
			return
		}
		var request struct {
			Model string `json:"model"`
		}
		if err := json.NewDecoder(r.Body).Decode(&request); err != nil {
			t.Error(err)
		}
		if request.Model == "bad" {
			w.WriteHeader(404)
			_, _ = w.Write([]byte(`{"error":"model not found"}`))
			return
		}
		_, _ = w.Write([]byte(`{"choices":[{"message":{"content":"good"}}]}`))
	}))
	defer server.Close()
	t.Setenv("MIXLAYER_API_KEY", "dummy")
	t.Setenv("MIXLAYER_BASE_URL", server.URL+"/v1")
	t.Setenv("MIXLAYER_PLATFORM_URL", server.URL)
	t.Setenv("MIXLAYER_DATA_DIR", t.TempDir())
	t.Setenv("MIXLAYER_CONFIG_DIR", t.TempDir())
	flags := &rootFlags{timeout: 3 * time.Second}
	out, err := runCommand(t, newLadderCmd(flags), "Hello", "--rungs", "good,bad", "--db", filepath.Join(t.TempDir(), "data.db"))
	if err == nil || ExitCode(err) != 6 || !strings.Contains(out, "good") || !strings.Contains(out, "model not found") {
		t.Fatalf("err=%v output=%s", err, out)
	}
}

func TestEscalateAllFailHasNoClaimedSavings(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(404)
		_, _ = w.Write([]byte(`{"error":"model not found"}`))
	}))
	defer server.Close()
	t.Setenv("MIXLAYER_API_KEY", "dummy")
	t.Setenv("MIXLAYER_BASE_URL", server.URL+"/v1")
	t.Setenv("MIXLAYER_PLATFORM_URL", server.URL)
	t.Setenv("MIXLAYER_DATA_DIR", t.TempDir())
	t.Setenv("MIXLAYER_CONFIG_DIR", t.TempDir())
	flags := &rootFlags{timeout: 3 * time.Second}
	out, err := runCommand(t, newEscalateCmd(flags), "Hello", "--rungs", "bad", "--db", filepath.Join(t.TempDir(), "data.db"))
	var result map[string]any
	parseErr := json.Unmarshal([]byte(out), &result)
	if err == nil || ExitCode(err) != 3 || parseErr != nil || result["saved_usd"] != nil || result["frontier_baseline_usd"] != nil || result["baseline_reason"] == "" {
		t.Fatalf("err=%v parse=%v result=%#v", err, parseErr, result)
	}
}
