// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

package client

import (
	"context"
	"errors"
	"fmt"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/mixlayer/cli/internal/config"
)

func TestGetModelLooksUpSlashIDsInList(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/models" {
			w.WriteHeader(http.StatusNotFound)
			return
		}
		fmt.Fprint(w, `{"object":"list","data":[{"id":"qwen/qwen3.5-9b","object":"model"},{"id":"z-ai/glm-5.3","object":"model"}]}`)
	}))
	defer srv.Close()
	c := New(&config.Config{BaseURL: srv.URL}, 5*time.Second, 0)
	c.NoCache = true
	got, err := c.GetModel(context.Background(), "z-ai/glm-5.3")
	if err != nil || !strings.Contains(string(got), `"z-ai/glm-5.3"`) {
		t.Fatalf("got %s err %v", got, err)
	}
	_, err = c.GetModel(context.Background(), "no/such")
	var apiErr *APIError
	if !errors.As(err, &apiErr) || apiErr.StatusCode != http.StatusNotFound {
		t.Fatalf("missing model: %v", err)
	}
}
