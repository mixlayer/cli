// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.
package mixlayerskill

import "embed"

//go:generate sh -c "GOPROXY=off go run ../../cmd/mixlayer agent skill-ref > references/commands.md"

// Files contains the installable Agent Skill.
//
//go:embed SKILL.md references/*.md
var Files embed.FS

func Text() string {
	b, _ := Files.ReadFile("SKILL.md")
	return string(b)
}
