// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

package cli

import (
	"github.com/spf13/cobra"
)

func newModelsCmd(flags *rootFlags) *cobra.Command {
	cmd := &cobra.Command{
		Use:         "models",
		Short:       "Browse the model catalog",
		Annotations: map[string]string{"mcp:read-only": "true"},
		RunE:        parentNoSubcommandRunE(flags),
	}

	cmd.AddCommand(newModelsGetCmd(flags))
	cmd.AddCommand(newModelsListCmd(flags))
	cmd.AddCommand(newModelsQueryCmd(flags))
	return cmd
}
