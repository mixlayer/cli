// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

package main

import (
	"os"

	"github.com/mixlayer/cli/internal/cli"
)

func main() {
	if err := cli.Execute(); err != nil {
		os.Exit(cli.ExitCode(err))
	}
}
