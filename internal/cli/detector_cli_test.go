package cli

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/mixlayer/cli/internal/mixapi"
	"github.com/spf13/cobra"
)

func TestLocalDetectorRequestOptionsAndJSONModeFallback(t *testing.T) {
	calls := 0
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		calls++
		var req mixapi.ChatRequest
		if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
			t.Error(err)
		}
		if req.Temperature == nil || *req.Temperature != 0 || req.ReasoningEffort != "none" || len(req.Messages) != 2 || req.Messages[0].Role != "system" || !strings.HasPrefix(req.Messages[0].Content, "/no_think") || req.Messages[1].Role != "user" {
			t.Errorf("request=%#v", req)
		}
		if calls == 1 {
			if string(req.ResponseFormat) != `{"type":"json_object"}` {
				t.Errorf("response format=%s", req.ResponseFormat)
			}
			w.WriteHeader(400)
			_, _ = w.Write([]byte(`{"error":"unsupported response_format"}`))
			return
		}
		if len(req.ResponseFormat) != 0 {
			t.Errorf("fallback response format=%s", req.ResponseFormat)
		}
		_, _ = w.Write([]byte(`{"choices":[{"message":{"content":"{\"entities\":[]}"}}]}`))
	}))
	defer server.Close()
	d := detectorFlags{Name: "local:fake", BaseURL: server.URL + "/v1", Timeout: time.Second}
	runtime, err := d.runtime(&cobra.Command{}, &rootFlags{timeout: 2 * time.Second})
	if err != nil {
		t.Fatal(err)
	}
	got, err := runtime.Call(context.Background(), `Return strict JSON. Text: "Hello"`)
	if err != nil || got != `{"entities":[]}` || calls != 2 {
		t.Fatalf("got=%q err=%v calls=%d", got, err, calls)
	}
}

func TestLocalDetectorHonorsDetectorTimeout(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		select {
		case <-r.Context().Done():
		case <-time.After(200 * time.Millisecond):
		}
	}))
	defer server.Close()
	d := detectorFlags{Name: "local:fake", BaseURL: server.URL + "/v1", Timeout: 20 * time.Millisecond}
	runtime, err := d.runtime(&cobra.Command{}, &rootFlags{timeout: time.Second})
	if err != nil {
		t.Fatal(err)
	}
	start := time.Now()
	_, err = runtime.Call(context.Background(), `Return strict JSON. Text: "Hello"`)
	if err == nil || time.Since(start) > 300*time.Millisecond {
		t.Fatalf("err=%v elapsed=%s", err, time.Since(start))
	}
}
