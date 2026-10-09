// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

package cli

import "github.com/spf13/cobra"

// version is the CLI's version, overridable at build time via ldflags.
var version = "0.1.0-dev"

// newVersionCmd prints the CLI name and version.
func newVersionCmd(flags *rootFlags) *cobra.Command {
	return &cobra.Command{
		Use:     "version",
		Short:   "Print version",
		Example: "  mixlayer version\n  mixlayer version --agent",
		RunE: func(cmd *cobra.Command, args []string) error {
			if flags.asJSON || flags.agent {
				return flags.printJSON(cmd, map[string]string{"name": cmd.Root().Name(), "version": version})
			}
			_, err := cmd.OutOrStdout().Write([]byte(cmd.Root().Name() + " " + version + "\n"))
			return err
		},
	}
}
