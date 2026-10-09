// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.
package mcp

import (
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
)

func TestClaudeDesktopManifestBinaryAndSensitiveKey(t *testing.T) {
	b, err := os.ReadFile(filepath.Join("..", "..", "manifest.json"))
	if err != nil {
		t.Fatal(err)
	}
	var manifest struct {
		Server struct {
			Type       string `json:"type"`
			EntryPoint string `json:"entry_point"`
			MCPConfig  struct {
				Command string            `json:"command"`
				Env     map[string]string `json:"env"`
			} `json:"mcp_config"`
		} `json:"server"`
		UserConfig map[string]struct {
			Sensitive bool `json:"sensitive"`
			Required  bool `json:"required"`
		} `json:"user_config"`
	}
	if err := json.Unmarshal(b, &manifest); err != nil {
		t.Fatal(err)
	}
	if manifest.Server.Type != "binary" || manifest.Server.EntryPoint != "bin/mixlayer-mcp" {
		t.Fatalf("wrong server binary: %+v", manifest.Server)
	}
	if manifest.Server.MCPConfig.Command != "${__dirname}/bin/mixlayer-mcp" {
		t.Fatalf("wrong MCP command: %q", manifest.Server.MCPConfig.Command)
	}
	if manifest.Server.MCPConfig.Env["MIXLAYER_API_KEY"] != "${user_config.mixlayer_api_key}" {
		t.Fatal("MCP key must reference sensitive user_config")
	}
	key := manifest.UserConfig["mixlayer_api_key"]
	if !key.Sensitive || !key.Required {
		t.Fatalf("API key config is not sensitive and required: %+v", key)
	}
}
