// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.
package cli

import (
	"strings"
	"testing"

	"github.com/spf13/cobra"
)

func TestDecideAgentDiscovery(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	t.Setenv("MIXLAYER_ENABLE_JINI", "1")
	root := RootCmd()
	context := buildAgentContext(root)
	found := false
	for _, cmd := range context.Commands {
		if cmd.Name == "decide" {
			for _, sub := range cmd.Subcommands {
				if sub.Name == "batch" {
					found = true
				}
			}
		}
	}
	if !found {
		t.Fatal("agent-context omits decide batch")
	}
	found = false
	for _, code := range context.ExitCodes {
		if code.Code == 9 {
			found = true
		}
	}
	if !found {
		t.Fatal("agent-context omits exit 9")
	}
	for _, command := range []string{"decide", "decide batch"} {
		found = false
		for _, entry := range whichIndex {
			if entry.Command == command {
				found = true
			}
		}
		if !found {
			t.Fatalf("which omits %s", command)
		}
	}
	if got := rankWhich(activeWhichIndex(), "classify JSONL", 3); len(got) == 0 || !strings.Contains(got[0].Entry.Command, "decide") {
		t.Fatalf("which results=%+v", got)
	}
}

func TestDecideHiddenUnlessEnabled(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	t.Setenv("MIXLAYER_ENABLE_JINI", "")
	root := RootCmd()
	for _, cmd := range buildAgentContext(root).Commands {
		if cmd.Name == "decide" {
			t.Fatal("agent-context lists decide without MIXLAYER_ENABLE_JINI")
		}
	}
	for _, entry := range activeWhichIndex() {
		if strings.HasPrefix(entry.Command, "decide") {
			t.Fatalf("which lists %s without MIXLAYER_ENABLE_JINI", entry.Command)
		}
	}
	var decide, escalate *cobra.Command
	for _, c := range root.Commands() {
		switch c.Name() {
		case "decide":
			decide = c
		case "escalate":
			escalate = c
		}
	}
	if decide == nil || !decide.Hidden || !decide.Runnable() {
		t.Fatal("decide must stay registered, runnable and hidden")
	}
	if f := escalate.Flags().Lookup("checker-model"); f == nil || !f.Hidden {
		t.Fatal("escalate --checker-model must be hidden")
	}
	if strings.Contains(strings.ToLower(escalate.Example+escalate.Flags().Lookup("checker").Usage), "jini") {
		t.Fatal("escalate help mentions Jini")
	}
}
