// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

package cli

import (
	"errors"
	"io"
	"strings"
	"testing"

	"github.com/mixlayer/cli/internal/client"
	"github.com/spf13/cobra"
)

var agentEnvNames = []string{"CLAUDECODE", "CLAUDE_CODE", "CODEX_SANDBOX", "CURSOR_AGENT", "GEMINI_CLI", "AI_AGENT"}

func TestAgentAutoDetection(t *testing.T) {
	cases := []struct {
		name         string
		env          string
		optOut       string
		args         []string
		wantJSON     bool
		wantDefaults bool
	}{
		{name: "no agent", args: []string{"probe"}},
		{name: "opt out", env: "CLAUDECODE", optOut: "1", args: []string{"probe"}},
		{name: "explicit agent despite opt out", optOut: "1", args: []string{"probe", "--agent"}, wantJSON: true, wantDefaults: true},
		{name: "human friendly wins", env: "CLAUDECODE", args: []string{"probe", "--human-friendly"}},
		{name: "json false wins", env: "CLAUDECODE", args: []string{"probe", "--json=false"}, wantDefaults: true},
	}
	for _, env := range agentEnvNames {
		cases = append(cases, struct {
			name         string
			env          string
			optOut       string
			args         []string
			wantJSON     bool
			wantDefaults bool
		}{name: env, env: env, args: []string{"probe"}, wantJSON: true, wantDefaults: true})
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			for _, name := range agentEnvNames {
				t.Setenv(name, "")
			}
			t.Setenv("MIXLAYER_NO_AGENT_DETECT", tc.optOut)
			if tc.env != "" {
				t.Setenv(tc.env, "1")
			}
			noColor, humanFriendly = false, false
			t.Cleanup(func() { noColor, humanFriendly = false, false })
			var flags rootFlags
			root := newRootCmd(&flags)
			root.AddCommand(&cobra.Command{Use: "probe", RunE: func(cmd *cobra.Command, args []string) error { return nil }})
			root.SetOut(io.Discard)
			root.SetErr(io.Discard)
			root.SetArgs(tc.args)
			if err := root.Execute(); err != nil {
				t.Fatal(err)
			}
			if flags.asJSON != tc.wantJSON {
				t.Errorf("json = %v, want %v", flags.asJSON, tc.wantJSON)
			}
			if flags.compact != tc.wantDefaults || flags.noInput != tc.wantDefaults || flags.yes != tc.wantDefaults || noColor != tc.wantDefaults {
				t.Errorf("agent defaults = compact:%v no-input:%v yes:%v no-color:%v, want all %v", flags.compact, flags.noInput, flags.yes, noColor, tc.wantDefaults)
			}
		})
	}
}

func TestGenericSurfacesRemoved(t *testing.T) {
	root := RootCmd()
	for _, name := range []string{"import", "sync", "search", "workflow", "export", "feedback", "api"} {
		if command, _, err := root.Find([]string{name}); err == nil && command != root && command.Name() == name {
			t.Errorf("%s should not be registered", name)
		}
	}
	for _, name := range []string{"deliver", "data-source", "max-age", "idempotent", "allow-partial-failure"} {
		if root.PersistentFlags().Lookup(name) != nil {
			t.Errorf("--%s should not exist", name)
		}
	}
}

func TestHTTPExitCodes(t *testing.T) {
	cases := []struct {
		status int
		code   int
		hint   string
	}{
		{402, 8, "do not retry"},
		{429, 7, "Retry-After"},
		{401, 4, "check your token"},
		{404, 3, "mixlayer models list"},
		{500, 5, ""},
	}
	for _, tc := range cases {
		err := &client.APIError{Method: "GET", Path: "/models", StatusCode: tc.status, Body: "test"}
		classified := classifyAPIError(err, nil)
		if code := ExitCode(classified); code != tc.code {
			t.Errorf("HTTP %d: code %d, want %d", tc.status, code, tc.code)
		}
		if !errors.Is(classified, err) {
			t.Errorf("HTTP %d: original error lost", tc.status)
		}
		if tc.hint != "" && !strings.Contains(classified.Error(), tc.hint) {
			t.Errorf("HTTP %d: missing hint %q", tc.status, tc.hint)
		}
	}
}
