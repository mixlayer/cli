// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

package client

import (
	"context"
	"net"
	"net/http"
	"net/http/httptest"
	"sync/atomic"
	"testing"
	"time"

	"github.com/mixlayer/cli/internal/config"
)

// Regression for patch ambiguous-write-retries-disabled: a POST that fails
// ambiguously (5xx or dropped connection) must be sent exactly once, while a
// GET still retries.
func TestAmbiguousWriteFailureIsNotReplayed(t *testing.T) {
	t.Setenv("HTTPS_PROXY", "")
	t.Setenv("HTTP_PROXY", "")
	for _, mode := range []string{"502", "drop"} {
		t.Run(mode, func(t *testing.T) {
			var hits atomic.Int32
			srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				hits.Add(1)
				if mode == "502" {
					http.Error(w, "bad gateway", http.StatusBadGateway)
					return
				}
				hj, _ := w.(http.Hijacker)
				conn, _, _ := hj.Hijack()
				if tcp, ok := conn.(*net.TCPConn); ok {
					_ = tcp.SetLinger(0)
				}
				_ = conn.Close()
			}))
			defer srv.Close()
			c := New(&config.Config{BaseURL: srv.URL}, 5*time.Second, 0)
			c.NoCache = true
			_, _, err := c.Post(context.Background(), "/v1/things", map[string]any{"a": 1})
			if err == nil {
				t.Fatal("expected an error")
			}
			if got := hits.Load(); got != 1 {
				t.Fatalf("POST sent %d times, want exactly 1", got)
			}
		})
	}
}

func TestReadsStillRetryAfterServerError(t *testing.T) {
	var hits atomic.Int32
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if hits.Add(1) == 1 {
			http.Error(w, "bad gateway", http.StatusBadGateway)
			return
		}
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{"ok":true}`))
	}))
	defer srv.Close()
	c := New(&config.Config{BaseURL: srv.URL}, 5*time.Second, 0)
	c.NoCache = true
	if _, err := c.Get(context.Background(), "/v1/things", nil); err != nil {
		t.Fatalf("GET: %v", err)
	}
	if got := hits.Load(); got < 2 {
		t.Fatalf("GET sent %d times, want a retry after 502", got)
	}
}
