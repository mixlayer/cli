// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.
package client

import (
	"context"
	"errors"
	"fmt"
	"net/http"
	"net/http/httptest"
	"sync/atomic"
	"testing"
	"time"

	"github.com/mixlayer/cli/internal/config"
	"github.com/mixlayer/cli/internal/systemone"
)

func TestSystemOneRetryBudgetAndCancellation(t *testing.T) {
	for _, header := range []string{"120", "999999999999999999999999999999999", time.Now().Add(time.Hour).UTC().Format(http.TimeFormat), ""} {
		var calls atomic.Int32
		server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
			calls.Add(1)
			w.Header().Set("Retry-After", header)
			w.WriteHeader(529)
			fmt.Fprint(w, `{"error":{"code":"overloaded","message":"No classification worker is currently available"}}`)
		}))
		c := New(&config.Config{BaseURL: server.URL}, time.Second, 0)
		ctx, cancel := context.WithTimeout(context.Background(), 50*time.Millisecond)
		_, err := c.SystemOne(ctx, systemone.Request{Model: "fixture", State: systemone.JSON("facts"), Questions: systemone.Questions{"q": {Type: "noul", Instructions: systemone.JSON("?")}}})
		cancel()
		server.Close()
		if calls.Load() != 1 || err == nil {
			t.Fatalf("calls=%d err=%v", calls.Load(), err)
		}
		if header == "" && !errors.Is(err, context.DeadlineExceeded) {
			t.Fatalf("want cancellation: %v", err)
		}
	}
}

func TestSystemOneURL(t *testing.T) {
	for _, input := range []string{"https://example.invalid", "https://example.invalid/v1", "https://example.invalid/v1/systemone"} {
		got, err := SystemOneURL(input)
		if err != nil || got != "https://example.invalid/v1/systemone" {
			t.Fatalf("got=%s err=%v", got, err)
		}
	}
	for _, input := range []string{"ftp://example.invalid", "https://user:pass@example.invalid", "https://example.invalid/?key=bad", "bad"} {
		if _, err := SystemOneURL(input); err == nil {
			t.Fatalf("accepted %s", input)
		}
	}
}
