package cli

import (
	"bytes"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"
	"time"
)

func TestDoctorMarksInvalidKeyFromModelsProbe(t *testing.T) {
	var rootCalls, modelCalls int
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/v1/":
			rootCalls++
			_, _ = w.Write([]byte(`{"ok":true}`))
		case "/v1/models":
			modelCalls++
			w.WriteHeader(401)
		default:
			t.Errorf("unexpected %s", r.URL.Path)
		}
	}))
	defer server.Close()
	t.Setenv("MIXLAYER_API_KEY", "dummy")
	t.Setenv("MIXLAYER_BASE_URL", server.URL+"/v1")
	var flags rootFlags
	root := newRootCmd(&flags)
	var out bytes.Buffer
	root.SetOut(&out)
	root.SetArgs([]string{"--home", t.TempDir(), "doctor", "--json"})
	if err := root.Execute(); err != nil {
		t.Fatal(err)
	}
	var report map[string]any
	if err := json.Unmarshal(out.Bytes(), &report); err != nil {
		t.Fatal(err)
	}
	if report["auth"] != "invalid" || rootCalls != 1 || modelCalls != 1 {
		t.Fatalf("report=%#v root=%d models=%d", report, rootCalls, modelCalls)
	}
}

func TestDoctorProbeUsesOneAttemptWithinTimeout(t *testing.T) {
	calls := 0
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		calls++
		select {
		case <-r.Context().Done():
		case <-time.After(200 * time.Millisecond):
		}
	}))
	defer server.Close()
	t.Setenv("MIXLAYER_API_KEY", "dummy")
	t.Setenv("MIXLAYER_BASE_URL", server.URL+"/v1")
	var flags rootFlags
	root := newRootCmd(&flags)
	var out bytes.Buffer
	root.SetOut(&out)
	root.SetArgs([]string{"--home", t.TempDir(), "--timeout", "40ms", "doctor", "--json"})
	start := time.Now()
	if err := root.Execute(); err != nil {
		t.Fatal(err)
	}
	if calls != 1 || time.Since(start) > time.Second {
		t.Fatalf("calls=%d elapsed=%s output=%s", calls, time.Since(start), out.String())
	}
}
