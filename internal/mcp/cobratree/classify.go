// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

package cobratree

import (
	"strconv"
	"strings"

	"github.com/spf13/cobra"
)

const (
	EndpointAnnotation = "cli:endpoint"
	HiddenAnnotation   = "mcp:hidden"
	// ReadOnlyAnnotation, when set on a Cobra command to "true"/"1"/"yes",
	// causes the runtime walker to register the resulting MCP tool with
	// readOnlyHint=true. Use for novel CLI commands that don't mutate
	// external state — read-only API queries, local cache reads, etc.
	// Without it, hosts like Claude Desktop default to "could write or
	// delete" and demand permission per call.
	ReadOnlyAnnotation = "mcp:read-only"
	// PositionalWriteSinksAnnotation lists zero-based positional argument
	// indexes that write to user-visible files when populated. It is enforced
	// only on commands that also carry ReadOnlyAnnotation.
	PositionalWriteSinksAnnotation = "mcp:write-positionals"
)

type commandKind int

const (
	commandNovel commandKind = iota
	commandEndpoint
	commandFramework
	commandHidden
)

// frameworkCommands are top-level CLI commands the walker should skip when
// mirroring the Cobra tree. Two cases qualify:
//
//  1. A typed MCP tool already covers the same capability (the typed tool's
//     schema is strictly better than a shell-out). Examples: `sql`,
//     `context`/`about`/`agent-context`.
//  2. The command is non-functional via MCP (interactive setup, shell-only
//     ergonomics, trivial introspection). Examples:
//     `auth`, `completion`, `doctor`, `version`, `profile`,
//     `which`, `help`.
//
// Adding a new command means deciding which of the two cases above applies.
// When in doubt, leave it out of this list: the walker registers any
// user-facing command as a shell-out tool.
var frameworkCommands = map[string]bool{
	"about":         true,
	"agent-context": true,
	"auth":          true,
	"completion":    true,
	"doctor":        true,
	"help":          true,
	"profile":       true,
	"sql":           true,
	"version":       true,
	"which":         true,
}

func classify(cmd *cobra.Command) commandKind {
	if cmd == nil || cmd.Hidden || isMCPHidden(cmd) {
		return commandHidden
	}
	if endpointID(cmd) != "" {
		return commandEndpoint
	}
	if isTopLevelFrameworkCommand(cmd) {
		return commandFramework
	}
	return commandNovel
}

func isTopLevelFrameworkCommand(cmd *cobra.Command) bool {
	if cmd == nil || !frameworkCommands[cmd.Name()] {
		return false
	}
	parent := cmd.Parent()
	return parent != nil && parent.Parent() == nil
}

func endpointID(cmd *cobra.Command) string {
	if cmd == nil || cmd.Annotations == nil {
		return ""
	}
	return strings.TrimSpace(cmd.Annotations[EndpointAnnotation])
}

// isMCPHidden reports whether cmd or any ancestor opts out of MCP, so hiding
// a command group hides every subcommand too.
func isMCPHidden(cmd *cobra.Command) bool {
	for c := cmd; c != nil; c = c.Parent() {
		if annotationIsTrue(c, HiddenAnnotation) {
			return true
		}
	}
	return false
}

func isMCPReadOnly(cmd *cobra.Command) bool {
	return annotationIsTrue(cmd, ReadOnlyAnnotation)
}

func positionalWriteSinkIndexes(cmd *cobra.Command) map[int]bool {
	if cmd == nil || cmd.Annotations == nil {
		return nil
	}
	raw := strings.TrimSpace(cmd.Annotations[PositionalWriteSinksAnnotation])
	if raw == "" {
		return nil
	}
	out := map[int]bool{}
	for _, part := range strings.FieldsFunc(raw, func(r rune) bool {
		return r == ',' || r == ';' || r == ' ' || r == '\t' || r == '\n'
	}) {
		idx, err := strconv.Atoi(strings.TrimSpace(part))
		if err != nil || idx < 0 {
			continue
		}
		out[idx] = true
	}
	if len(out) == 0 {
		return nil
	}
	return out
}

func annotationIsTrue(cmd *cobra.Command, key string) bool {
	if cmd == nil || cmd.Annotations == nil {
		return false
	}
	v := strings.ToLower(strings.TrimSpace(cmd.Annotations[key]))
	return v == "true" || v == "1" || v == "yes"
}
