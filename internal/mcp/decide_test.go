// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.
package mcp

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"os/exec"
	"path/filepath"
	"strings"
	"sync/atomic"
	"testing"

	mcplib "github.com/mark3labs/mcp-go/mcp"
	"github.com/mark3labs/mcp-go/server"
	"github.com/mixlayer/cli/internal/systemone"
)

func TestMCPDecideInlineWorkflow(t *testing.T) {
	t.Setenv("MIXLAYER_ENABLE_JINI", "1")
	bin := filepath.Join(t.TempDir(), "mixlayer-fixture")
	build := exec.Command("go", "build", "-o", bin, "../../cmd/mixlayer")
	if out, err := build.CombinedOutput(); err != nil {
		t.Fatalf("build: %v\n%s", err, out)
	}
	resetMCPPathEnv(t)
	t.Setenv("MIXLAYER_CLI_PATH", bin)
	t.Setenv("MIXLAYER_API_KEY", "")
	var calls atomic.Int32
	fake := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		calls.Add(1)
		var req systemone.Request
		if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
			t.Error(err)
		}
		if string(req.State) != `"Facts with --flags and $(literal)"` || req.Questions["q"].Type != "noul" {
			t.Errorf("request=%+v", req)
		}
		fmt.Fprint(w, `{"model":"fixture","answers":{"q":{"type":"noul","noul":0.9}},"usage":{"input_tokens":12,"output_tokens":0}}`)
	}))
	defer fake.Close()
	t.Setenv("MIXLAYER_BASE_URL", fake.URL+"/v1")
	s := server.NewMCPServer("test", "0", server.WithToolCapabilities(false))
	RegisterTools(s)
	entry := s.ListTools()["decide"]
	if entry == nil || !contains(entry.Tool.InputSchema.Required, "state_text") || !contains(entry.Tool.InputSchema.Required, "questions") {
		t.Fatal("missing inline schema")
	}
	if entry.Tool.InputSchema.Properties["questions"].(map[string]any)["type"] != "object" {
		t.Fatal("questions must be JSON object")
	}
	args := map[string]any{"state_text": "Facts with --flags and $(literal)", "questions": map[string]any{"q": map[string]any{"type": "noul", "instructions": "Valid?"}}, "model": "fixture", "dry_run": true}
	var req mcplib.CallToolRequest
	req.Params.Arguments = args
	handler := workflowHandler("decide")
	result, err := handler(context.Background(), req)
	if err != nil || result.IsError || calls.Load() != 0 {
		t.Fatalf("dry result=%+v err=%v calls=%d", result, err, calls.Load())
	}
	var preview struct {
		Body systemone.Request
		URL  string
	}
	if err := json.Unmarshal([]byte(result.Content[0].(mcplib.TextContent).Text), &preview); err != nil || preview.Body.Model != "fixture" || preview.URL != fake.URL+"/v1/systemone" {
		t.Fatalf("preview=%+v err=%v", preview, err)
	}
	args["dry_run"] = false
	result, err = handler(context.Background(), req)
	if err != nil || result.IsError || calls.Load() != 1 {
		t.Fatalf("live fake result=%+v err=%v", result, err)
	}
	args["fail_if"] = []any{"q>=0.9"}
	result, err = handler(context.Background(), req)
	if err != nil || !result.IsError || len(result.Content) != 2 {
		t.Fatalf("gate result=%+v err=%v", result, err)
	}
	if !strings.Contains(result.Content[0].(mcplib.TextContent).Text, `"answers"`) || !strings.Contains(result.Content[1].(mcplib.TextContent).Text, "exit status 9") {
		t.Fatalf("gate lost answers or exit status: %+v", result)
	}
	args["questions"] = "local-file.json"
	result, err = handler(context.Background(), req)
	if err != nil || !result.IsError || calls.Load() != 2 {
		t.Fatalf("invalid arguments result=%+v err=%v", result, err)
	}
}
