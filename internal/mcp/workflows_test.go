// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.
package mcp

import (
	"context"
	"strings"
	"testing"

	mcplib "github.com/mark3labs/mcp-go/mcp"
	"github.com/mark3labs/mcp-go/server"
)

func TestCoreWorkflowToolListAnnotationsAndDescriptions(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	s := server.NewMCPServer("Mixlayer test", "0", server.WithToolCapabilities(false))
	RegisterTools(s)
	tools := s.ListTools()
	for name, readOnly := range map[string]bool{
		"ask": false, "ladder": false, "escalate": false,
		"shield_redact": false, "shield_ask": false,
		"models_list": true, "doctor": true, "context": true,
	} {
		entry := tools[name]
		if entry == nil {
			t.Errorf("tools/list missing %s", name)
			continue
		}
		tool := entry.Tool
		if len(tool.Description) < 40 {
			t.Errorf("%s description too short: %q", name, tool.Description)
		}
		ann := tool.Annotations
		if ann.ReadOnlyHint == nil || *ann.ReadOnlyHint != readOnly {
			t.Errorf("%s readOnlyHint incorrect", name)
		}
		if ann.DestructiveHint == nil || *ann.DestructiveHint {
			t.Errorf("%s destructiveHint incorrect", name)
		}
		if ann.OpenWorldHint == nil || *ann.OpenWorldHint != (name != "context") {
			t.Errorf("%s openWorldHint incorrect", name)
		}
	}
	for name := range tools {
		if strings.HasPrefix(name, "vault_") {
			t.Errorf("vault operation exposed through MCP: %s", name)
		}
	}
	for _, name := range []string{"shield_ask", "shield_redact"} {
		if _, ok := tools[name].Tool.InputSchema.Properties["dry_run"]; !ok {
			t.Errorf("%s missing dry_run parameter", name)
		}
	}
	if !contains(tools["shield_ask"].Tool.InputSchema.Required, "upstream") {
		t.Fatal("shield_ask must require an explicit upstream")
	}
}

func TestMCPContextIncludesEmbeddedSkill(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	result, err := handleContext(context.Background(), mcplib.CallToolRequest{})
	if err != nil {
		t.Fatal(err)
	}
	if len(result.Content) == 0 || !strings.Contains(result.Content[0].(mcplib.TextContent).Text, "Never print vault values") {
		t.Fatal("MCP context does not include the skill invariants")
	}
}

func contains(items []string, want string) bool {
	for _, item := range items {
		if item == want {
			return true
		}
	}
	return false
}

func TestJiniToolsHiddenUnlessEnabled(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	t.Setenv("MIXLAYER_ENABLE_JINI", "")
	s := server.NewMCPServer("Mixlayer test", "0", server.WithToolCapabilities(false))
	RegisterTools(s)
	tools := s.ListTools()
	if tools["decide"] != nil {
		t.Fatal("decide tool listed without MIXLAYER_ENABLE_JINI")
	}
	if _, ok := tools["escalate"].Tool.InputSchema.Properties["checker_model"]; ok {
		t.Fatal("escalate exposes Jini checker without MIXLAYER_ENABLE_JINI")
	}

	t.Setenv("MIXLAYER_ENABLE_JINI", "1")
	s = server.NewMCPServer("Mixlayer test", "0", server.WithToolCapabilities(false))
	RegisterTools(s)
	tools = s.ListTools()
	if tools["decide"] == nil {
		t.Fatal("decide tool missing with MIXLAYER_ENABLE_JINI=1")
	}
	if _, ok := tools["escalate"].Tool.InputSchema.Properties["checker_model"]; !ok {
		t.Fatal("escalate lacks Jini checker with MIXLAYER_ENABLE_JINI=1")
	}
}

func TestMCPSQLRejectsPrivateTables(t *testing.T) {
	for _, q := range []string{
		"SELECT value FROM vault",
		`select * from "vault"`,
		"SELECT term FROM shield_dictionary",
		"WITH v AS (SELECT * FROM [vault]) SELECT * FROM v",
		"SELECT sql FROM sqlite_master",
		"SELECT * FROM pragma_table_info('vault')",
	} {
		if err := rejectPrivateTables(q); err == nil {
			t.Errorf("%q: expected rejection", q)
		}
	}
	for _, q := range []string{
		"SELECT model, sum(cost_usd) FROM runs GROUP BY model",
		"SELECT * FROM audit",
		"SELECT vaulted_at FROM runs",
	} {
		if err := rejectPrivateTables(q); err != nil {
			t.Errorf("%q: unexpected rejection: %v", q, err)
		}
	}
	var req mcplib.CallToolRequest
	req.Params.Arguments = map[string]any{"query": "SELECT value FROM vault"}
	res, err := handleSQL(context.Background(), req)
	if err != nil || !res.IsError {
		t.Fatalf("handleSQL vault: res=%+v err=%v", res, err)
	}
}

func TestMCPHidesPrivateAndSetupTools(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	s := server.NewMCPServer("Mixlayer test", "0", server.WithToolCapabilities(false))
	RegisterTools(s)
	tools := s.ListTools()
	for name := range tools {
		if strings.HasPrefix(name, "shield_dict") || strings.HasPrefix(name, "agent") || strings.HasPrefix(name, "vault") || name == "search" {
			t.Errorf("MCP exposes %s", name)
		}
	}
}
