package upstream

import (
	"context"
	"encoding/json"
	"errors"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/mixlayer/cli/internal/client"
	"github.com/mixlayer/cli/internal/mixapi"
)

func TestPresetsAndKeys(t *testing.T) {
	cases := []struct{ name, base, env, host string }{
		{"mixlayer", "https://mixlayer.ai/v1", "MIXLAYER_API_KEY", "mixlayer.ai"},
		{"openai", "https://api.openai.com/v1", "OPENAI_API_KEY", "api.openai.com"},
		{"anthropic", "https://api.anthropic.com/v1", "ANTHROPIC_API_KEY", "api.anthropic.com"},
		{"openrouter", "https://openrouter.ai/api/v1", "OPENROUTER_API_KEY", "openrouter.ai"},
	}
	for _, tc := range cases {
		got, err := Resolve(tc.name, "", "")
		if err != nil || got.BaseURL != tc.base || got.KeyEnv != tc.env || got.Host != tc.host {
			t.Fatalf("%s: %#v, %v", tc.name, got, err)
		}
	}
	if _, err := Resolve("custom", "", ""); err == nil {
		t.Fatal("custom missing settings accepted")
	}
	if _, err := Resolve("custom", "https://user:secret@example.test/v1", "TEST_KEY"); err == nil {
		t.Fatal("URL userinfo accepted")
	}
	t.Setenv("TEST_MISSING_KEY", "")
	if _, err := KeyFromEnv("TEST_MISSING_KEY", false); err == nil || !strings.Contains(err.Error(), "TEST_MISSING_KEY") {
		t.Fatalf("missing key error = %v", err)
	}
}

func TestChatRetries429UsingRetryAfter(t *testing.T) {
	calls := 0
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		calls++
		if calls == 1 {
			w.Header().Set("Retry-After", "1")
			w.WriteHeader(429)
			_, _ = w.Write([]byte(`{"error":"slow down"}`))
			return
		}
		_, _ = w.Write([]byte(`{"choices":[{"message":{"content":"done"}}]}`))
	}))
	defer server.Close()
	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
	defer cancel()
	start := time.Now()
	_, err := Chat(ctx, server.Client(), server.URL, "dummy", "fake", []mixapi.Message{{Role: "user", Content: "hi"}})
	if err != nil || calls != 2 || time.Since(start) < time.Second {
		t.Fatalf("err=%v calls=%d elapsed=%s", err, calls, time.Since(start))
	}
}

func TestChatHTTPErrorKeepsBodyAndRetryAfter(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Retry-After", "2")
		w.WriteHeader(402)
		_, _ = w.Write([]byte(`{"error":"insufficient prepaid credits"}`))
	}))
	defer server.Close()
	_, err := Chat(context.Background(), server.Client(), server.URL, "dummy", "fake", []mixapi.Message{{Role: "user", Content: "hi"}})
	var apiError *client.APIError
	if !errors.As(err, &apiError) || apiError.StatusCode != 402 || apiError.RetryAfter != "2" || !strings.Contains(apiError.Body, "insufficient prepaid credits") {
		t.Fatalf("err=%v", err)
	}
}

func TestChatCompatibleRequest(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/v1/chat/completions" || r.Header.Get("Authorization") != "Bearer dummy" {
			t.Errorf("request path/auth: %s %s", r.URL.Path, r.Header.Get("Authorization"))
		}
		var request mixapi.ChatRequest
		if err := json.NewDecoder(r.Body).Decode(&request); err != nil {
			t.Error(err)
		}
		if request.Model != "fake" || len(request.Messages) != 2 || request.Messages[0].Role != "system" {
			t.Errorf("request: %#v", request)
		}
		_, _ = w.Write([]byte(`{"choices":[{"message":{"content":"TOKEN_abcdef12"}}]}`))
	}))
	defer server.Close()
	result, err := Chat(context.Background(), server.Client(), server.URL+"/v1", "dummy", "fake", []mixapi.Message{{Role: "system", Content: "instruction"}, {Role: "user", Content: "question"}})
	if err != nil || result.Answer != "TOKEN_abcdef12" {
		t.Fatalf("result = %#v, %v", result, err)
	}
}
