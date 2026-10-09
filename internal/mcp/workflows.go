// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.
package mcp

import (
	"context"
	"encoding/json"
	"fmt"
	"math"
	"strings"

	"github.com/mixlayer/cli/internal/cliutil"

	mcplib "github.com/mark3labs/mcp-go/mcp"
	"github.com/mark3labs/mcp-go/server"
	"github.com/mixlayer/cli/internal/mcp/cobratree"
)

// Core workflow tools use small, typed schemas rather than the broad Cobra
// mirror. The shell-out executes the exact CLI policy and exit-code handling.
func registerCoreWorkflowTools(s *server.MCPServer) {
	add := func(name, desc string, readOnly bool, options []mcplib.ToolOption, handler server.ToolHandlerFunc) {
		base := []mcplib.ToolOption{
			mcplib.WithDescription(desc),
			mcplib.WithReadOnlyHintAnnotation(readOnly),
			mcplib.WithDestructiveHintAnnotation(false),
			mcplib.WithOpenWorldHintAnnotation(true),
		}
		s.AddTool(mcplib.NewTool(name, append(base, options...)...), handler)
	}
	question := mcplib.WithString("question", mcplib.Required(), mcplib.Description("The question or task to send to the selected model"))
	dryRun := mcplib.WithBoolean("dry_run", mcplib.Description("Preview the request without sending model data or spending credits"))
	model := mcplib.WithString("model", mcplib.Description("Model ID selected from the live models_list result"))
	if cliutil.JiniEnabled() {
		add("decide", "Evaluate typed Jini decisions from inline text and a questions JSON object; dry_run previews without sending data or spending credits. A matching fail_if condition returns an error with the decision response.", true,
			[]mcplib.ToolOption{mcplib.WithString("state_text", mcplib.Required(), mcplib.Description("Literal state text; no local file required")), mcplib.WithObject("questions", mcplib.Required(), mcplib.Description("Systemone questions map keyed by ID, each with type, instructions and optional criteria")), mcplib.WithString("model", mcplib.Description("Classification model; falls back to classification_model config, with no silent default")), mcplib.WithArray("fail_if", mcplib.WithStringItems(), mcplib.Description("Conditions such as dup>0.8 or route==review; any match exits 9")), dryRun}, workflowHandler("decide"))
	}
	add("ask", "Ask one live Mixlayer model and save its answer, token use, and cost in the local ledger. Preview before sending sensitive input.", false,
		[]mcplib.ToolOption{question, model, dryRun}, workflowHandler("ask"))
	add("ladder", "Run the same question across selected Mixlayer model rungs to compare answers, latency, reasoning, and cost.", false,
		[]mcplib.ToolOption{question, mcplib.WithString("rungs", mcplib.Description("Comma-separated live model IDs; default uses the live catalog")), mcplib.WithBoolean("reasoning", mcplib.Description("Request reasoning content when supported")), dryRun}, workflowHandler("ladder"))
	add("escalate", "Start with cheaper live models and climb only when an answer is insufficiently confident; save the trial ledger locally.", false,
		[]mcplib.ToolOption{question, mcplib.WithString("rungs", mcplib.Description("Comma-separated live model IDs; default uses the live catalog")), mcplib.WithNumber("confidence", mcplib.Description("Heuristic confidence threshold from 0 to 1")), dryRun}, workflowHandler("escalate"))
	if cliutil.JiniEnabled() {
		s.DeleteTools("escalate")
		add("escalate", "Start with cheaper live models and climb only when an answer is insufficiently confident; save the trial ledger locally.", false,
			[]mcplib.ToolOption{question, mcplib.WithString("rungs", mcplib.Description("Comma-separated live model IDs; default uses the live catalog")), mcplib.WithNumber("confidence", mcplib.Description("Heuristic confidence threshold from 0 to 1")), mcplib.WithString("checker", mcplib.Description("heuristic (default) or jini")), mcplib.WithString("checker_model", mcplib.Description("Jini classification model; falls back to classification_model config")), mcplib.WithNumber("threshold", mcplib.Description("Jini minimum score from 0 to 1, default 0.8")), dryRun}, workflowHandler("escalate"))
	}
	add("shield_redact", "Mask personal data in a local file and populate the local reversible vault; dry run reports the planned operation without touching the vault.", false,
		[]mcplib.ToolOption{mcplib.WithString("file", mcplib.Required(), mcplib.Description("Path to the local file to redact")), mcplib.WithString("detector", mcplib.Description("regex, local:<model>, or mixlayer:<model>")), dryRun}, workflowHandler("shield_redact"))
	add("shield_ask", "Mask question and file data before an explicit upstream model call, run the outbound tripwire, and return a local restoration receipt.", false,
		[]mcplib.ToolOption{question, mcplib.WithString("data", mcplib.Description("Optional local data file path")), mcplib.WithString("upstream", mcplib.Required(), mcplib.Description("Explicit upstream: mixlayer, openai, anthropic, openrouter, or custom")), mcplib.WithString("upstream_base_url", mcplib.Description("Base URL for a custom upstream")), mcplib.WithString("upstream_key_env", mcplib.Description("Name of the environment variable holding a custom upstream key; never pass its value")), model, mcplib.WithString("detector", mcplib.Description("regex, local:<model>, or mixlayer:<model>; use a model for external upstreams")), dryRun}, workflowHandler("shield_ask"))
	add("models_list", "List currently available Mixlayer models and live prices when the platform provides them; use this before selecting a model.", true,
		[]mcplib.ToolOption{mcplib.WithBoolean("live_prices", mcplib.Description("Fetch live organization prices when available; defaults to true")), mcplib.WithString("select", mcplib.Description("Comma-separated output fields")), dryRun}, workflowHandler("models_list"))
	add("doctor", "Check Mixlayer authentication, API reachability, and resolved local paths; report missing credentials without revealing them.", true,
		[]mcplib.ToolOption{}, workflowHandler("doctor"))
}

func workflowHandler(name string) server.ToolHandlerFunc {
	return func(ctx context.Context, req mcplib.CallToolRequest) (*mcplib.CallToolResult, error) {
		args := req.GetArguments()
		if args == nil {
			args = map[string]any{}
		}
		cliArgs := []string{"--agent"}
		if name == "decide" {
			state, err := requiredString(args, "state_text")
			if err != nil {
				return mcplib.NewToolResultError(err.Error()), nil
			}
			questions, ok := args["questions"].(map[string]any)
			if !ok {
				return mcplib.NewToolResultError("questions must be an inline JSON object"), nil
			}
			body, err := json.Marshal(questions)
			if err != nil {
				return mcplib.NewToolResultError("questions must be JSON serializable"), nil
			}
			cliArgs = append(cliArgs, "--state-text", state, "--questions-json", string(body))
			if raw, present := args["fail_if"]; present {
				conditions, ok := raw.([]any)
				if !ok {
					return mcplib.NewToolResultError("fail_if must be an array of strings"), nil
				}
				for _, raw := range conditions {
					condition, ok := raw.(string)
					if !ok {
						return mcplib.NewToolResultError("fail_if must contain strings"), nil
					}
					cliArgs = append(cliArgs, "--fail-if", condition)
				}
			}
		}
		var positional string
		switch name {
		case "ask", "ladder", "escalate", "shield_ask":
			v, err := requiredString(args, "question")
			if err != nil {
				return mcplib.NewToolResultError(err.Error()), nil
			}
			positional = v
		case "shield_redact":
			v, err := requiredString(args, "file")
			if err != nil {
				return mcplib.NewToolResultError(err.Error()), nil
			}
			positional = v
		}
		if name == "shield_ask" {
			if _, err := requiredString(args, "upstream"); err != nil {
				return mcplib.NewToolResultError(err.Error()), nil
			}
		}
		if name == "shield_redact" {
			if dry, _ := args["dry_run"].(bool); dry {
				return toolResultJSON(map[string]any{"dry_run": true, "operation": "shield redact", "file": positional, "detector": stringArg(args, "detector", "regex"), "network_calls": 0, "vault_writes": 0})
			}
		}
		if dry, _ := args["dry_run"].(bool); dry {
			cliArgs = append(cliArgs, "--dry-run")
		}
		switch name {
		case "decide":
			cliArgs = append(cliArgs, "decide")
		case "ask":
			cliArgs = append(cliArgs, "ask")
		case "ladder":
			cliArgs = append(cliArgs, "ladder")
		case "escalate":
			cliArgs = append(cliArgs, "escalate")
		case "shield_redact":
			cliArgs = append(cliArgs, "shield", "redact")
		case "shield_ask":
			cliArgs = append(cliArgs, "shield", "ask")
		case "models_list":
			cliArgs = append(cliArgs, "models", "list")
		case "doctor":
			cliArgs = append(cliArgs, "doctor")
		default:
			return mcplib.NewToolResultError("unknown workflow"), nil
		}
		for _, key := range []string{"model", "rungs", "detector", "data", "upstream", "upstream_base_url", "upstream_key_env", "select", "checker", "checker_model"} {
			if v, ok := args[key]; ok {
				str, ok := v.(string)
				if !ok {
					return mcplib.NewToolResultError(fmt.Sprintf("%s must be a string", key)), nil
				}
				if strings.TrimSpace(str) != "" {
					cliArgs = append(cliArgs, "--"+strings.ReplaceAll(key, "_", "-"), str)
				}
			}
		}
		for _, key := range []string{"confidence", "threshold"} {
			if v, ok := args[key]; ok {
				num, ok := v.(float64)
				if !ok || math.IsNaN(num) || num < 0 || num > 1 {
					return mcplib.NewToolResultError(key + " must be a number from 0 to 1"), nil
				}
				cliArgs = append(cliArgs, "--"+key, fmt.Sprint(num))
			}
		}
		if v, _ := args["reasoning"].(bool); v {
			cliArgs = append(cliArgs, "--reasoning")
		}
		if name == "models_list" {
			if v, present := args["live_prices"]; !present || v == true {
				cliArgs = append(cliArgs, "--live-prices")
			}
		}
		if positional != "" {
			cliArgs = append(cliArgs, "--", positional)
		}
		bin, err := cobratree.SiblingCLIPath()
		if err != nil {
			return mcplib.NewToolResultError("companion mixlayer CLI not found: " + err.Error()), nil
		}
		out, err := cobratree.RunCLICommand(ctx, bin, cliArgs)
		if err != nil {
			if name == "decide" && strings.TrimSpace(out) != "" {
				return &mcplib.CallToolResult{IsError: true, Content: []mcplib.Content{mcplib.TextContent{Type: "text", Text: out}, mcplib.TextContent{Type: "text", Text: err.Error()}}}, nil
			}
			return mcplib.NewToolResultError(err.Error()), nil
		}
		return mcplib.NewToolResultText(out), nil
	}
}

func requiredString(args map[string]any, key string) (string, error) {
	v, ok := args[key].(string)
	if !ok || strings.TrimSpace(v) == "" {
		return "", fmt.Errorf("%s is required", key)
	}
	return v, nil
}

func stringArg(args map[string]any, key, fallback string) string {
	v, ok := args[key].(string)
	if !ok || v == "" {
		return fallback
	}
	return v
}
