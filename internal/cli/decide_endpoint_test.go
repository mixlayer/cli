// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.
package cli

import (
	"net/http"
	"net/http/httptest"
	"sync/atomic"
	"testing"
)

// A foreign --endpoint must never receive the configured Mixlayer credential.
func TestDecideForeignEndpointDropsMixlayerKey(t *testing.T) {
	var mixlayerHits atomic.Int32
	mixlayer := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		mixlayerHits.Add(1)
		decisionReply(w, "fixture")
	}))
	defer mixlayer.Close()
	var gotAuth atomic.Value
	gotAuth.Store("unset")
	foreign := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		gotAuth.Store(r.Header.Get("Authorization"))
		decisionReply(w, "fixture")
	}))
	defer foreign.Close()
	decisionEnv(t, mixlayer.URL+"/v1")
	t.Setenv("MIXLAYER_API_KEY", "mxl-test-should-not-leak")

	// Host ports differ, so httptest servers are distinct hosts.
	if _, err := runCommand(t, newDecideCmd(decisionFlags()), "--model", "fixture", "--state-text", "s", "--noul", "q=Q?", "--endpoint", foreign.URL+"/v1"); err != nil {
		t.Fatal(err)
	}
	if got := gotAuth.Load().(string); got != "" {
		t.Fatalf("foreign endpoint received Authorization %q", got)
	}

	t.Setenv("SELFHOST_KEY", "local-key")
	if _, err := runCommand(t, newDecideCmd(decisionFlags()), "--model", "fixture", "--state-text", "s", "--noul", "q=Q?", "--endpoint", foreign.URL+"/v1", "--endpoint-key-env", "SELFHOST_KEY"); err != nil {
		t.Fatal(err)
	}
	if got := gotAuth.Load().(string); got != "Bearer local-key" {
		t.Fatalf("foreign endpoint Authorization = %q, want explicit env key", got)
	}

	// Same host as the configured base URL keeps the Mixlayer credential.
	if _, err := runCommand(t, newDecideCmd(decisionFlags()), "--model", "fixture", "--state-text", "s", "--noul", "q=Q?", "--endpoint", mixlayer.URL+"/v1"); err != nil {
		t.Fatal(err)
	}
	if mixlayerHits.Load() != 1 {
		t.Fatalf("mixlayer hits = %d", mixlayerHits.Load())
	}
}
