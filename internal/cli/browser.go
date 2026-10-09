// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

package cli

import (
	"context"
	"os/exec"
	"runtime"
)

const (
	apiKeysURL = "https://console.mixlayer.com/app/api-keys"
	signUpURL  = "https://console.mixlayer.com/sign-up"
)

// openURL opens url in the user's default browser.
func openURL(url string) error {
	var cmd *exec.Cmd
	switch runtime.GOOS {
	case "darwin":
		cmd = exec.CommandContext(context.Background(), "open", url)
	case "windows":
		cmd = exec.CommandContext(context.Background(), "rundll32", "url.dll,FileProtocolHandler", url)
	default:
		cmd = exec.CommandContext(context.Background(), "xdg-open", url)
	}
	return cmd.Start()
}
