package cli

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"regexp"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	"github.com/mixlayer/cli/internal/mixapi"
	"github.com/mixlayer/cli/internal/store"
)

var testTokenRE = regexp.MustCompile(`[A-Z_]+_[0-9a-f]{8}`)

func executeShieldAsk(t *testing.T, args ...string) (map[string]any, error) {
	t.Helper()
	var out, stderr bytes.Buffer
	flags := &rootFlags{asJSON: true, timeout: time.Second * 5}
	filtered := make([]string, 0, len(args))
	for _, arg := range args {
		if arg == "--dry-run" {
			flags.dryRun = true
		} else {
			filtered = append(filtered, arg)
		}
	}
	cmd := newShieldAskCmd(flags)
	cmd.SetOut(&out)
	cmd.SetErr(&stderr)
	cmd.SetArgs(filtered)
	err := cmd.Execute()
	if err != nil {
		return nil, err
	}
	var result map[string]any
	if err := json.Unmarshal(out.Bytes(), &result); err != nil {
		t.Fatalf("json output: %v; %s", err, out.String())
	}
	return result, nil
}

func TestShieldAskRoundTripReceiptAndDryRun(t *testing.T) {
	var calls atomic.Int32
	frontier := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		calls.Add(1)
		var req mixapi.ChatRequest
		if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
			t.Error(err)
			return
		}
		if r.URL.Path != "/v1/chat/completions" || r.Header.Get("Authorization") != "Bearer dummy" || len(req.Messages) != 2 || req.Messages[0].Role != "system" {
			t.Errorf("request: %#v", req)
		}
		if strings.Contains(req.Messages[1].Content, "Zora Quill") || strings.Contains(req.Messages[1].Content, "zora@fiction.example") {
			t.Error("raw PII reached frontier")
		}
		tokens := testTokenRE.FindAllString(req.Messages[1].Content, -1)
		answer := "No token"
		if len(tokens) > 0 {
			answer = "Reply to " + strings.Join(tokens, " and ")
		}
		_ = json.NewEncoder(w).Encode(map[string]any{"choices": []any{map[string]any{"message": map[string]any{"content": answer}}}})
	}))
	defer frontier.Close()
	t.Setenv("TEST_FRONTIER_KEY", "dummy")
	dir := t.TempDir()
	dataFile := filepath.Join(dir, "data.txt")
	if err := os.WriteFile(dataFile, []byte("Zora Quill: zora@fiction.example"), 0600); err != nil {
		t.Fatal(err)
	}
	args := []string{"Draft a reply", "--data", dataFile, "--upstream", "custom", "--accept-regex-only", "--upstream-base-url", frontier.URL + "/v1", "--upstream-key-env", "TEST_FRONTIER_KEY", "--model", "fake", "--db", filepath.Join(dir, "vault.db")}
	preview, err := executeShieldAsk(t, append(append([]string{}, args...), "--dry-run")...)
	if err != nil {
		t.Fatal(err)
	}
	if calls.Load() != 0 || !strings.HasPrefix(fmt.Sprint(preview["destination_host"]), "127.0.0.1:") || strings.Contains(fmt.Sprint(preview["payload"]), "Zora Quill") {
		t.Fatalf("dry run: calls=%d preview=%#v", calls.Load(), preview)
	}
	result, err := executeShieldAsk(t, args...)
	if err != nil {
		t.Fatal(err)
	}
	if calls.Load() != 1 || !strings.Contains(fmt.Sprint(result["answer"]), "zora@fiction.example") {
		t.Fatalf("round trip: calls=%d result=%#v", calls.Load(), result)
	}
	s, err := store.Open(filepath.Join(dir, "vault.db"))
	if err != nil {
		t.Fatal(err)
	}
	defer s.Close()
	audits, err := s.AuditRecords(context.Background(), "", 10)
	if err != nil {
		t.Fatal(err)
	}
	if len(audits) != 1 || !strings.HasPrefix(audits[0].UpstreamHost, "127.0.0.1:") || audits[0].RestoredCount < 1 || audits[0].CostUSD != nil || audits[0].PayloadSHA256 == "" {
		t.Fatalf("audit = %#v", audits)
	}
	run, err := s.GetRun(context.Background(), audits[0].RunID)
	if err != nil || !run.CostUnknown {
		t.Fatalf("run cost = %#v, %v", run, err)
	}
	if strings.Contains(run.Prompt, "Zora Quill") || strings.Contains(run.Prompt, "zora@fiction.example") || !strings.Contains(run.Prompt, "PERSON_") || audits[0].TripwireDetector != "regex" {
		t.Fatalf("stored prompt or tripwire = %q, %q", run.Prompt, audits[0].TripwireDetector)
	}
}

func TestShieldAskNearMissReceipt(t *testing.T) {
	frontier := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		var req mixapi.ChatRequest
		if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
			t.Error(err)
			return
		}
		token := testTokenRE.FindString(req.Messages[1].Content)
		if token == "" {
			t.Error("no token received")
			return
		}
		answer := strings.ToLower(token) + " EMAIL_deadbeef"
		_ = json.NewEncoder(w).Encode(map[string]any{"choices": []map[string]any{{"message": map[string]string{"content": answer}}}})
	}))
	defer frontier.Close()
	t.Setenv("TEST_FRONTIER_KEY", "dummy")
	db := filepath.Join(t.TempDir(), "vault.db")
	result, err := executeShieldAsk(t, "Email zora@fiction.example", "--upstream", "custom", "--accept-regex-only", "--upstream-base-url", frontier.URL+"/v1", "--upstream-key-env", "TEST_FRONTIER_KEY", "--model", "fake", "--db", db)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(fmt.Sprint(result["answer"]), "zora@fiction.example") || result["restored_count"] != float64(1) || result["unrestored_count"] != float64(1) {
		t.Fatalf("result = %#v", result)
	}
	s, err := store.Open(db)
	if err != nil {
		t.Fatal(err)
	}
	defer s.Close()
	audits, err := s.AuditRecords(context.Background(), "", 10)
	if err != nil {
		t.Fatal(err)
	}
	if len(audits) != 1 || len(audits[0].Repairs) != 1 || audits[0].UnrestoredCount != 1 || len(audits[0].UnrestoredTokens) != 1 {
		t.Fatalf("audit = %#v", audits)
	}
}

func TestShieldAskMissingModelAndKey(t *testing.T) {
	t.Setenv("TEST_ABSENT_KEY", "")
	db := filepath.Join(t.TempDir(), "db.sqlite")
	base := []string{"Hello", "--upstream", "custom", "--upstream-base-url", "http://127.0.0.1:12345/v1", "--upstream-key-env", "TEST_ABSENT_KEY", "--db", db}
	if _, err := executeShieldAsk(t, base...); err == nil || ExitCode(err) != 2 {
		t.Fatalf("missing model error = %v", err)
	}
	if _, err := executeShieldAsk(t, append(base, "--model", "fake", "--accept-regex-only")...); err == nil || ExitCode(err) != 4 || !strings.Contains(err.Error(), "TEST_ABSENT_KEY") {
		t.Fatalf("missing key error = %v", err)
	}
}

func TestShieldAskModelDetectorFailClosedAndFallback(t *testing.T) {
	var frontierCalls atomic.Int32
	frontier := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		frontierCalls.Add(1)
		_, _ = w.Write([]byte(`{"choices":[{"message":{"content":"Done"}}]}`))
	}))
	defer frontier.Close()
	detector := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		_, _ = w.Write([]byte(`{"choices":[{"message":{"content":"invalid detector JSON"}}]}`))
	}))
	defer detector.Close()
	t.Setenv("TEST_FRONTIER_KEY", "dummy")
	db := filepath.Join(t.TempDir(), "vault.db")
	args := []string{"Email zora@fiction.example", "--upstream", "custom", "--upstream-base-url", frontier.URL + "/v1", "--upstream-key-env", "TEST_FRONTIER_KEY", "--model", "fake", "--detector", "local:fake", "--detector-base-url", detector.URL + "/v1", "--db", db}
	if _, err := executeShieldAsk(t, args...); err == nil || frontierCalls.Load() != 0 {
		t.Fatalf("fail closed: err=%v calls=%d", err, frontierCalls.Load())
	}
	if _, err := executeShieldAsk(t, append(args, "--detector-fallback", "regex")...); err != nil {
		t.Fatal(err)
	}
	if frontierCalls.Load() != 1 {
		t.Fatalf("frontier calls = %d", frontierCalls.Load())
	}
	s, err := store.Open(db)
	if err != nil {
		t.Fatal(err)
	}
	defer s.Close()
	audits, err := s.AuditRecords(context.Background(), "", 10)
	if err != nil {
		t.Fatal(err)
	}
	if len(audits) != 1 || audits[0].DetectorFallback != "regex" || !strings.HasPrefix(audits[0].DetectorHost, "127.0.0.1:") {
		t.Fatalf("audit = %#v", audits)
	}
}

func TestShieldAskModelDetectorMasksReturnedSpans(t *testing.T) {
	var detectorCalls, frontierCalls atomic.Int32
	detector := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		detectorCalls.Add(1)
		var req mixapi.ChatRequest
		if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
			t.Error(err)
			return
		}
		if detectorCalls.Load() == 1 && !strings.Contains(req.Messages[1].Content, "123 Fictional Lane") {
			t.Error("detector did not receive raw text")
		}
		_ = json.NewEncoder(w).Encode(map[string]any{"choices": []map[string]any{{"message": map[string]string{"content": `{"entities":[{"kind":"ADDRESS","text":"123 Fictional Lane"},{"kind":"PERSON","text":"not present"}]}`}}}})
	}))
	defer detector.Close()
	frontier := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		frontierCalls.Add(1)
		var req mixapi.ChatRequest
		if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
			t.Error(err)
			return
		}
		if strings.Contains(req.Messages[1].Content, "123 Fictional Lane") || !strings.Contains(req.Messages[1].Content, "ADDRESS_") {
			t.Errorf("frontier got %q", req.Messages[1].Content)
		}
		_ = json.NewEncoder(w).Encode(map[string]any{"choices": []map[string]any{{"message": map[string]string{"content": "Okay"}}}})
	}))
	defer frontier.Close()
	t.Setenv("TEST_FRONTIER_KEY", "dummy")
	_, err := executeShieldAsk(t, "Summarize 123 Fictional Lane", "--upstream", "custom", "--upstream-base-url", frontier.URL+"/v1", "--upstream-key-env", "TEST_FRONTIER_KEY", "--model", "fake", "--detector", "local:fake", "--detector-base-url", detector.URL+"/v1", "--db", filepath.Join(t.TempDir(), "vault.db"))
	if err != nil || detectorCalls.Load() == 0 || frontierCalls.Load() != 1 {
		t.Fatalf("calls detector=%d frontier=%d err=%v", detectorCalls.Load(), frontierCalls.Load(), err)
	}
}

func TestShieldAskTripwireFailsClosedOnResidualName(t *testing.T) {
	var detectorCalls, frontierCalls atomic.Int32
	detector := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		entities := `[]`
		if detectorCalls.Add(1) == 2 {
			entities = `[{"kind":"PERSON","text":"milo paperboat"}]`
		}
		_ = json.NewEncoder(w).Encode(map[string]any{"choices": []map[string]any{{"message": map[string]string{"content": `{"entities":` + entities + `}`}}}})
	}))
	defer detector.Close()
	frontier := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		frontierCalls.Add(1)
		_, _ = w.Write([]byte(`{"choices":[{"message":{"content":"wrong"}}]}`))
	}))
	defer frontier.Close()
	t.Setenv("TEST_FRONTIER_KEY", "dummy")
	_, err := executeShieldAsk(t, "Summarize milo paperboat", "--upstream", "custom", "--upstream-base-url", frontier.URL+"/v1", "--upstream-key-env", "TEST_FRONTIER_KEY", "--model", "fake", "--detector", "local:fake", "--detector-base-url", detector.URL+"/v1", "--db", filepath.Join(t.TempDir(), "vault.db"))
	if err == nil || ExitCode(err) != 6 || frontierCalls.Load() != 0 || detectorCalls.Load() != 2 {
		t.Fatalf("err=%v frontier=%d detector=%d", err, frontierCalls.Load(), detectorCalls.Load())
	}
}

func TestShieldAskRegexOnlyExternalRequiresOptIn(t *testing.T) {
	_, err := executeShieldAsk(t, "Hello", "--upstream", "openai", "--model", "fake", "--db", filepath.Join(t.TempDir(), "vault.db"))
	if err == nil || ExitCode(err) != 2 || !strings.Contains(err.Error(), "--accept-regex-only") {
		t.Fatalf("err=%v", err)
	}
}

func TestShieldEvalRegexVsFakeModelAndThreshold(t *testing.T) {
	samplePath := filepath.Join("..", "..", "testdata", "shield-eval", "synthetic.jsonl")
	samples, err := readEvalSamples(samplePath)
	if err != nil {
		t.Fatal(err)
	}
	byText := map[string][]evalGold{}
	for _, s := range samples {
		byText[s.Text] = s.Entities
	}
	fake := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		var req mixapi.ChatRequest
		if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
			t.Error(err)
			return
		}
		prompt := req.Messages[1].Content
		idx := strings.LastIndex(prompt, "Text: ")
		if idx < 0 {
			t.Error("no text in detector prompt")
			return
		}
		var sampleText string
		if err := json.Unmarshal([]byte(prompt[idx+6:]), &sampleText); err != nil {
			t.Error(err)
			return
		}
		var entities []evalGold
		for _, g := range byText[sampleText] {
			if _, ok := map[string]bool{"PERSON": true, "ADDRESS": true, "ORG": true, "LOCATION": true, "DATE_OF_BIRTH": true, "ID_NUMBER": true, "MEDICAL": true, "ACCOUNT": true, "OTHER": true}[g.Kind]; ok {
				entities = append(entities, g)
			}
		}
		if entities == nil {
			entities = []evalGold{}
		}
		content, _ := json.Marshal(map[string]any{"entities": entities})
		_ = json.NewEncoder(w).Encode(map[string]any{"choices": []map[string]any{{"message": map[string]string{"content": string(content)}}}})
	}))
	defer fake.Close()
	var out bytes.Buffer
	flags := &rootFlags{asJSON: true, timeout: 5 * time.Second}
	cmd := newShieldEvalCmd(flags)
	cmd.SetOut(&out)
	cmd.SetArgs([]string{"--sample", samplePath, "--detector", "regex", "--detector", "local:fake", "--detector-base-url", fake.URL + "/v1", "--min-recall", "0"})
	if err := cmd.Execute(); err != nil {
		t.Fatal(err)
	}
	var report struct {
		Results []struct {
			Detector string      `json:"detector"`
			Overall  evalMetrics `json:"overall"`
		} `json:"results"`
	}
	if err := json.Unmarshal(out.Bytes(), &report); err != nil {
		t.Fatal(err)
	}
	if len(report.Results) != 2 || report.Results[1].Overall.Recall <= report.Results[0].Overall.Recall {
		t.Fatalf("eval = %#v", report)
	}
	out.Reset()
	cmd = newShieldEvalCmd(flags)
	cmd.SetOut(&out)
	cmd.SetArgs([]string{"--sample", samplePath, "--detector", "regex", "--min-recall", "0.9"})
	if err := cmd.Execute(); err == nil || ExitCode(err) != 2 {
		t.Fatalf("threshold error = %v", err)
	}
}

func TestShieldDictionaryCommands(t *testing.T) {
	path := filepath.Join(t.TempDir(), "vault.db")
	flags := &rootFlags{asJSON: true}
	run := func(args ...string) string {
		t.Helper()
		var out bytes.Buffer
		cmd := newShieldDictCmd(flags)
		cmd.SetOut(&out)
		cmd.SetArgs(append([]string{"--db", path}, args...))
		if err := cmd.Execute(); err != nil {
			t.Fatal(err)
		}
		return out.String()
	}
	if got := run("add", "moon pebble", "--kind", "PERSON"); !strings.Contains(got, "moon pebble") {
		t.Fatal(got)
	}
	if got := run("list"); !strings.Contains(got, "moon pebble") {
		t.Fatal(got)
	}
	if got := run("remove", "moon pebble"); !strings.Contains(got, "true") {
		t.Fatal(got)
	}
	if got := run("list"); strings.Contains(got, "moon pebble") {
		t.Fatal(got)
	}
}

func TestSavingsExcludesUnknownUpstreamCost(t *testing.T) {
	path := filepath.Join(t.TempDir(), "runs.db")
	s, err := store.Open(path)
	if err != nil {
		t.Fatal(err)
	}
	if err := s.SaveRun(context.Background(), store.RunRecord{ID: "known", Command: "ask", Prompt: "known", CostUSD: 0.25, TotalTokens: 100}); err != nil {
		t.Fatal(err)
	}
	if err := s.SaveRun(context.Background(), store.RunRecord{ID: "unknown", Command: "shield ask", Prompt: "unknown", CostUnknown: true, TotalTokens: 100}); err != nil {
		t.Fatal(err)
	}
	s.Close()
	var out bytes.Buffer
	cmd := newSavingsCmd(&rootFlags{asJSON: true})
	cmd.SetOut(&out)
	cmd.SetArgs([]string{"--db", path})
	if err := cmd.Execute(); err != nil {
		t.Fatal(err)
	}
	var result map[string]any
	if err := json.Unmarshal(out.Bytes(), &result); err != nil {
		t.Fatal(err)
	}
	if result["runs"] != float64(1) || result["excluded_unknown_cost_runs"] != float64(1) || result["actual_usd"] != 0.25 {
		t.Fatalf("savings = %#v", result)
	}
}
