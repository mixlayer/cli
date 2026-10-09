// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

package config

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/mixlayer/cli/internal/cliutil"
	"github.com/pelletier/go-toml/v2"
)

type Config struct {
	ClassificationModel string            `toml:"classification_model,omitempty"`
	BaseURL             string            `toml:"base_url"`
	AuthHeaderVal       string            `toml:"auth_header"`
	Headers             map[string]string `toml:"headers,omitempty"`
	AuthSource          string            `toml:"-"`
	CredentialSource    string            `toml:"-"`
	// configOwner records which on-disk file parseConfigData populated this
	// config from ("config-kind path" or "legacy config path") so the
	// credential-source fallback below reports where config-stored
	// credentials actually live. Unexported: never persisted.
	configOwner string
	// legacySourcePath records the legacy config path when Load fell
	// back to it. Used by save() to scrub credential fields from the
	// old location after relocation. Unexported: never persisted.
	legacySourcePath string
	AccessToken      string          `toml:"access_token"`
	RefreshToken     string          `toml:"refresh_token"`
	TokenExpiry      time.Time       `toml:"token_expiry"`
	ClientID         string          `toml:"client_id"`
	ClientSecret     string          `toml:"client_secret"`
	Path             string          `toml:"-"`
	envOverrides     map[string]bool `toml:"-"`
	fileConfig       *Config         `toml:"-"`
	MixlayerApiKey   string          `toml:"api_key"`
}

func Load(configPath string) (*Config, error) {
	cfg := &Config{
		BaseURL: "https://mixlayer.ai/v1",
	}

	// Resolve config path
	path, explicitConfigFile, err := resolveConfigPath(configPath)
	if err != nil {
		return nil, err
	}
	cfg.Path = path

	if explicitConfigFile {
		if err := readConfigFile(path, cfg, "config-kind path"); err != nil && !os.IsNotExist(err) {
			return nil, err
		}
	} else {
		legacyPath, err := LegacyConfigPath()
		if err != nil {
			return nil, err
		}
		data, sourcePath, err := cliutil.ReadFileWithLegacyFallback(path, legacyPath)
		if err != nil {
			if !os.IsNotExist(err) {
				return nil, err
			}
		} else {
			owner := "config-kind path"
			if sourcePath == legacyPath {
				owner = "legacy config path"
			}
			parsed := *cfg
			if err := parseConfigData(data, &parsed, sourcePath, owner); err != nil {
				if sourcePath == legacyPath {
					fmt.Fprintf(os.Stderr, "warning: legacy config parse skipped for %s: %v\n", sourcePath, err)
				} else {
					return nil, err
				}
			} else {
				*cfg = parsed
				if sourcePath == legacyPath {
					cfg.legacySourcePath = legacyPath
				}
			}
		}
	}
	cfg.Path = path
	creds, ok, err := cliutil.LoadCredentials()
	if err != nil {
		return nil, err
	}
	if ok && creds.HasValues() {
		cfg.clearCredentialFields()
		cfg.applyCredentials(creds)
		if cfg.hasCredentialFields() {
			cfg.AuthSource = "config"
			cfg.CredentialSource = "credentials file"
		}
	}

	cfg.snapshotFileConfig()

	// Env var overrides
	if v := os.Getenv("MIXLAYER_API_KEY"); v != "" {
		cfg.MixlayerApiKey = v
		cfg.markEnvOverride("MixlayerApiKey")
		cfg.AuthSource = "env:MIXLAYER_API_KEY"
		cfg.CredentialSource = "env:MIXLAYER_API_KEY"
	}
	// Label config-file-derived credentials so doctor can distinguish
	// "credentials persisted on disk" from "no credentials at all" — without
	// this, users who saved via set-token without an env var see a blank
	// auth_source and can't tell whether their config is being picked up.
	// The label is the literal "config" rather than "config:<path>"; the
	// config file path is exposed separately as report["config_path"], and
	// embedding it in auth_source leaks the user's home directory through
	// doctor's JSON envelope.
	if cfg.AuthSource == "" && cfg.hasCredentialFields() {
		cfg.AuthSource = "config"
	}
	if cfg.CredentialSource == "" && cfg.AuthSource == "config" {
		// Label config-stored credentials with the file they were parsed
		// from: the resolved config-kind path (covers --home and per-kind
		// env relocation as well as explicit --config files) or the legacy
		// config path when the read fell back to the pre-paths layout.
		cfg.CredentialSource = cfg.configOwner
		if cfg.CredentialSource == "" {
			cfg.CredentialSource = "legacy config path"
		}
	}

	// Base URL override for alternate deployments and local test servers.
	if v := os.Getenv("MIXLAYER_BASE_URL"); v != "" {
		cfg.BaseURL = v
	}
	return cfg, nil
}

func resolveConfigPath(configPath string) (string, bool, error) {
	if strings.TrimSpace(configPath) != "" {
		return configPath, true, nil
	}
	if path := os.Getenv("MIXLAYER_CONFIG"); path != "" {
		return path, true, nil
	}
	dir, err := cliutil.ConfigDir()
	if err != nil {
		return "", false, err
	}
	return filepath.Join(dir, "config.toml"), false, nil
}

func LegacyConfigPath() (string, error) {
	home, err := os.UserHomeDir()
	if err != nil {
		return "", fmt.Errorf("resolve legacy config path: %w", err)
	}
	// Non-XDG home default. When XDG_CONFIG_HOME relocates the active config,
	// this file is read as a fallback and scrubbed of secrets on auth writes.
	return filepath.Join(home, ".config", "mixlayer", "config.toml"), nil
}

func readConfigFile(path string, cfg *Config, owner string) error {
	data, err := os.ReadFile(path)
	if err != nil {
		return err
	}
	return parseConfigData(data, cfg, path, owner)
}

func parseConfigData(data []byte, cfg *Config, path string, owner string) error {
	if err := toml.Unmarshal(data, cfg); err != nil {
		return fmt.Errorf("parsing %s %s: %w", owner, path, err)
	}
	cfg.configOwner = owner
	return nil
}
func FileHasCredentialFields(path string) (bool, error) {
	var cfg Config
	if err := readConfigFile(path, &cfg, "credential probe"); err != nil {
		return false, err
	}
	return cfg.hasCredentialFields(), nil
}

func (c *Config) AuthHeader() string {
	if c.AuthHeaderVal != "" {
		return c.AuthHeaderVal
	}
	// Env-var token wins over file-stored AccessToken (env > config convention).
	if c.MixlayerApiKey != "" {
		if c.AuthSource == "" {
			c.AuthSource = "env:MIXLAYER_API_KEY"
		}
		return applyAuthFormat("Bearer {token}", map[string]string{
			"api_key":          c.MixlayerApiKey,
			"MIXLAYER_API_KEY": c.MixlayerApiKey,
			"token":            c.MixlayerApiKey,
		})
	}
	if c.AccessToken != "" {
		if c.AuthSource == "" || strings.HasPrefix(c.AuthSource, "env:") {
			c.AuthSource = "oauth2"
		}
		return applyAuthFormat("Bearer {token}", map[string]string{"access_token": c.AccessToken, "token": c.AccessToken})
	}
	return ""
}

func applyAuthFormat(format string, replacements map[string]string) string {
	if format == "" {
		return ""
	}
	for key, value := range replacements {
		format = strings.ReplaceAll(format, "{"+key+"}", value)
	}
	if strings.Contains(format, "{") {
		return ""
	}
	return format
}

func (c *Config) hasCredentialFields() bool {
	if c.AuthHeaderVal != "" ||
		c.AccessToken != "" ||
		c.RefreshToken != "" ||
		c.ClientID != "" ||
		c.ClientSecret != "" {
		return true
	}
	if c.MixlayerApiKey != "" {
		return true
	}
	return false
}

func (c *Config) clearCredentialFields() {
	c.AuthHeaderVal = ""
	c.AccessToken = ""
	c.RefreshToken = ""
	c.TokenExpiry = time.Time{}
	c.ClientID = ""
	c.ClientSecret = ""
	c.MixlayerApiKey = ""
}

func (c *Config) credentials() *cliutil.Credentials {
	return &cliutil.Credentials{
		AuthHeaderVal:  c.AuthHeaderVal,
		AccessToken:    c.AccessToken,
		RefreshToken:   c.RefreshToken,
		TokenExpiry:    c.TokenExpiry,
		ClientID:       c.ClientID,
		ClientSecret:   c.ClientSecret,
		MixlayerApiKey: c.MixlayerApiKey,
	}
}

func (c *Config) applyCredentials(creds *cliutil.Credentials) {
	if creds == nil {
		return
	}
	c.AuthHeaderVal = creds.AuthHeaderVal
	c.AccessToken = creds.AccessToken
	c.RefreshToken = creds.RefreshToken
	c.TokenExpiry = creds.TokenExpiry
	c.ClientID = creds.ClientID
	c.ClientSecret = creds.ClientSecret
	c.MixlayerApiKey = creds.MixlayerApiKey
}

func (c *Config) saveCredentialsFirst() error {
	persisted := c.configForSave()
	if err := cliutil.SaveCredentials(persisted.credentials()); err != nil {
		return err
	}
	c.CredentialSource = "credentials file"
	return nil
}

func (c *Config) SaveTokens(clientID, clientSecret, accessToken, refreshToken string, expiry time.Time) error {
	c.ClientID = clientID
	c.ClientSecret = clientSecret
	c.AccessToken = accessToken
	c.RefreshToken = refreshToken
	c.TokenExpiry = expiry
	delete(c.envOverrides, "ClientID")
	delete(c.envOverrides, "ClientSecret")
	delete(c.envOverrides, "AccessToken")
	delete(c.envOverrides, "RefreshToken")
	delete(c.envOverrides, "TokenExpiry")
	c.updateFileConfigField("ClientID")
	c.updateFileConfigField("ClientSecret")
	c.updateFileConfigField("AccessToken")
	c.updateFileConfigField("RefreshToken")
	c.updateFileConfigField("TokenExpiry")
	if err := c.saveCredentialsFirst(); err != nil {
		return err
	}
	return c.save()
}

func (c *Config) ClearTokens() error {
	// AuthHeader() falls back to the env-var-derived fields when AuthHeaderVal
	// and AccessToken are empty, so dropping the working credential requires
	// zeroing every emitted credential field, not just the OAuth trio.
	// ClientID/ClientSecret persist to disk via SaveTokens for the oauth2
	// and oauth2-cc flows, so logout must wipe them too; otherwise
	// `auth login` can re-mint a new access token unattended.
	c.AuthHeaderVal = ""
	c.AccessToken = ""
	c.RefreshToken = ""
	c.TokenExpiry = time.Time{}
	c.ClientID = ""
	c.ClientSecret = ""
	delete(c.envOverrides, "AuthHeaderVal")
	delete(c.envOverrides, "AccessToken")
	delete(c.envOverrides, "RefreshToken")
	delete(c.envOverrides, "TokenExpiry")
	delete(c.envOverrides, "ClientID")
	delete(c.envOverrides, "ClientSecret")
	c.updateFileConfigField("AuthHeaderVal")
	c.updateFileConfigField("AccessToken")
	c.updateFileConfigField("RefreshToken")
	c.updateFileConfigField("TokenExpiry")
	c.updateFileConfigField("ClientID")
	c.updateFileConfigField("ClientSecret")
	c.MixlayerApiKey = ""
	delete(c.envOverrides, "MixlayerApiKey")
	c.updateFileConfigField("MixlayerApiKey")
	if err := cliutil.RemoveCredentials(); err != nil {
		return err
	}
	return c.save()
}

func (c *Config) markEnvOverride(field string) {
	if c.envOverrides == nil {
		c.envOverrides = map[string]bool{}
	}
	c.envOverrides[field] = true
}

// cloneStringMap returns an independent copy of m (nil stays nil). The fileConfig
// snapshot must not share reference-type map fields (such as Headers) with the
// live config, or a later mutation to one would silently track in the other.
func cloneStringMap(m map[string]string) map[string]string {
	if m == nil {
		return nil
	}
	out := make(map[string]string, len(m))
	for k, v := range m {
		out[k] = v
	}
	return out
}

func (c *Config) snapshotFileConfig() {
	snapshot := *c
	snapshot.envOverrides = nil
	snapshot.fileConfig = nil
	// *c is a shallow copy: map fields are reference types, so the snapshot would
	// share them with c and silently track later mutations, defeating the
	// isolation this snapshot exists to provide. Clone them.
	snapshot.Headers = cloneStringMap(c.Headers)
	c.fileConfig = &snapshot
}

func (c *Config) configForSave() Config {
	out := *c
	if c.fileConfig != nil {
		if c.envOverrides["MixlayerApiKey"] {
			out.MixlayerApiKey = c.fileConfig.MixlayerApiKey
		}
	}
	out.envOverrides = nil
	out.fileConfig = nil
	return out
}

func (c *Config) updateFileConfigField(field string) {
	if c.fileConfig == nil || c.envOverrides[field] {
		return
	}
	switch field {
	case "AuthHeaderVal":
		c.fileConfig.AuthHeaderVal = c.AuthHeaderVal
	case "AccessToken":
		c.fileConfig.AccessToken = c.AccessToken
	case "RefreshToken":
		c.fileConfig.RefreshToken = c.RefreshToken
	case "TokenExpiry":
		c.fileConfig.TokenExpiry = c.TokenExpiry
	case "ClientID":
		c.fileConfig.ClientID = c.ClientID
	case "ClientSecret":
		c.fileConfig.ClientSecret = c.ClientSecret
	case "MixlayerApiKey":
		c.fileConfig.MixlayerApiKey = c.MixlayerApiKey
	}
}

func (c *Config) save() error {
	persisted := c.configForSave()
	data, err := toml.Marshal(persisted.persisted())
	if err != nil {
		return fmt.Errorf("marshaling config: %w", err)
	}
	if err := cliutil.AtomicWritePrivateFile(c.Path, data, 0o600, 0o700); err != nil {
		return err
	}
	c.scrubLegacyCredentials()
	persisted.clearCredentialFields()
	c.fileConfig = &persisted
	c.fileConfig.envOverrides = nil
	c.fileConfig.fileConfig = nil
	// persisted shares its map fields with c (configForSave shallow-copies *c),
	// so isolate the stored fileConfig the same way snapshotFileConfig does;
	// otherwise later mutations to c's maps leak into the on-disk snapshot.
	c.fileConfig.Headers = cloneStringMap(c.fileConfig.Headers)
	return nil
}
func (c *Config) scrubLegacyCredentials() {
	if c.legacySourcePath == "" || c.legacySourcePath == c.Path {
		return
	}
	data, err := os.ReadFile(c.legacySourcePath)
	if err != nil {
		if !os.IsNotExist(err) {
			fmt.Fprintf(os.Stderr, "warning: cannot read legacy config to scrub credentials: %v\n", err)
		}
		return
	}
	var legacy Config
	if err := toml.Unmarshal(data, &legacy); err != nil {
		fmt.Fprintf(os.Stderr, "warning: cannot parse legacy config to scrub credentials: %v\n", err)
		return
	}
	legacy.clearCredentialFields()
	scrubbed := legacy.persisted()
	scrubbedData, err := toml.Marshal(scrubbed)
	if err != nil {
		fmt.Fprintf(os.Stderr, "warning: cannot marshal scrubbed legacy config: %v\n", err)
		return
	}
	if err := cliutil.AtomicWritePrivateFile(c.legacySourcePath, scrubbedData, 0o600, 0o700); err != nil {
		fmt.Fprintf(os.Stderr, "warning: cannot write scrubbed legacy config: %v\n", err)
	}
}

type persistedConfig struct {
	BaseURL string            `toml:"base_url"`
	Headers map[string]string `toml:"headers,omitempty"`
}

func (c *Config) persisted() persistedConfig {
	return persistedConfig{
		BaseURL: c.BaseURL,
		Headers: c.Headers,
	}
}

// Ensure strings import is used
var _ = strings.ReplaceAll
