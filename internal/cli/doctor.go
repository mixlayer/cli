// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

package cli

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"io"
	"os"
	"strings"
	"time"

	"github.com/mixlayer/cli/internal/client"
	"github.com/mixlayer/cli/internal/cliutil"
	"github.com/mixlayer/cli/internal/config"
	"github.com/mixlayer/cli/internal/store"
	"github.com/spf13/cobra"
)

// looksLikeDoctorInterstitial reports whether the response body matches a known
// bot-detection challenge page (Cloudflare, Akamai, Vercel, AWS WAF, DataDome,
// PerimeterX). Only fires on the doctor probe — used to distinguish "transport
// reached the wall" from "transport failed entirely." Returns the vendor name
// when matched, or empty string when no match.
//
// Markers are anchored to <title> or vendor-specific strings to avoid
// false-positives on benign content. For example, a recipe titled "Just A
// Moment of Pause Cookies" must NOT match the Cloudflare challenge marker;
// only "<title>just a moment" (the actual interstitial title) does.
func looksLikeDoctorInterstitial(body []byte) string {
	if len(body) == 0 {
		return ""
	}
	limit := len(body)
	if limit > 8192 {
		limit = 8192
	}
	prefix := strings.ToLower(string(body[:limit]))
	if !strings.Contains(prefix, "<title") {
		// Every bot interstitial we recognize sets a <title>; bodies without
		// one are body-only API responses, not challenge pages.
		return ""
	}
	switch {
	case strings.Contains(prefix, "<title>just a moment") || // CF JS challenge
		strings.Contains(prefix, "challenges.cloudflare.com") || // CF Turnstile
		(strings.Contains(prefix, "attention required") && strings.Contains(prefix, "cloudflare")):
		return "Cloudflare"
	case strings.Contains(prefix, "akamai") && (strings.Contains(prefix, "request unsuccessful") || strings.Contains(prefix, "access denied")):
		return "Akamai"
	case strings.Contains(prefix, "x-vercel-mitigated") || strings.Contains(prefix, "x-vercel-challenge-token") ||
		(strings.Contains(prefix, "vercel") && strings.Contains(prefix, "challenge")):
		return "Vercel"
	case strings.Contains(prefix, "request blocked") && strings.Contains(prefix, "aws waf"):
		return "AWS WAF"
	case strings.Contains(prefix, "datadome") && (strings.Contains(prefix, "blocked") || strings.Contains(prefix, "captcha") || strings.Contains(prefix, "challenge")):
		return "DataDome"
	case strings.Contains(prefix, "perimeterx") || strings.Contains(prefix, "px-captcha"):
		return "PerimeterX"
	}
	return ""
}

func newDoctorCmd(flags *rootFlags) *cobra.Command {
	var failOn string
	cmd := &cobra.Command{
		Use:   "doctor",
		Short: "Check CLI health",
		Example: `  mixlayer doctor
  mixlayer doctor --json
  mixlayer doctor --fail-on warn
  mixlayer doctor --fail-on stale`,
		RunE: func(cmd *cobra.Command, args []string) error {
			report := map[string]any{}
			pathsReport := collectPathsReport()
			report["paths"] = pathsReport
			if warning := pathsWarning(pathsReport); warning != "" {
				report["paths_warning"] = warning
			}

			// Check config
			cfg, err := config.Load(flags.configPath)
			if err != nil {
				report["config"] = fmt.Sprintf("error: %s", err)
			} else {
				report["config"] = "ok"
				report["config_path"] = cfg.Path
				report["base_url"] = cfg.BaseURL
				collectCredentialsLocationReport(report, cfg)
			}

			// Check auth
			authConfigured := false
			if cfg != nil {
				header := cfg.AuthHeader()
				if header == "" {
					report["auth"] = "not configured"
					report["auth_hint"] = "Set it with: export MIXLAYER_API_KEY=\"your-key\" (create one at https://console.mixlayer.com/app/api-keys)"
				} else {
					authConfigured = true
					report["auth"] = "configured"
					report["auth_source"] = cfg.AuthSource
				}
			}

			// Check auth environment variables
			authEnvSet := []string{}
			authEnvRequiredMissing := []string{}
			authEnvInfo := []string{}
			authEnvOptionalNames := []string{}
			// Validation rejects multi-OR-group specs upstream, so the single optional-satisfied state is sufficient at runtime.
			authEnvOptionalSatisfied := false
			if os.Getenv("MIXLAYER_API_KEY") != "" {
				authEnvSet = append(authEnvSet, "MIXLAYER_API_KEY")
			} else if authConfigured {
				authSource, _ := report["auth_source"].(string)
				if authSource == "" {
					authSource = "config"
				}
				authEnvInfo = append(authEnvInfo, "credentials available from "+authSource)
			} else {
				authEnvRequiredMissing = append(authEnvRequiredMissing, "MIXLAYER_API_KEY")
			}
			switch {
			case len(authEnvRequiredMissing) > 0:
				report["env_vars"] = "ERROR missing required: " + strings.Join(authEnvRequiredMissing, ", ")
			case len(authEnvOptionalNames) > 1 && !authEnvOptionalSatisfied:
				report["env_vars"] = "INFO set one of: " + strings.Join(authEnvOptionalNames, " or ")
			case len(authEnvInfo) > 0 && authConfigured:
				report["env_vars"] = "OK " + strings.Join(authEnvInfo, "; ")
			case len(authEnvInfo) > 0:
				report["env_vars"] = "INFO " + strings.Join(authEnvInfo, "; ")
			default:
				report["env_vars"] = fmt.Sprintf("OK %d/%d available", len(authEnvSet), 1)
			}

			// Check API connectivity and validate credentials.
			//
			// The doctor uses the same client every other command uses --
			// flags.newClient() returns a *client.Client wrapping whatever
			// transport the spec declared (Surf for browser-chrome, stdlib
			// for standard). A separate stdlib http.Client would silently
			// bypass that choice and report false negatives against
			// Cloudflare-fronted, Akamai-fronted, or otherwise bot-detected
			// sites. By going through flags.newClient(), the doctor's
			// reachability verdict matches what real commands experience.
			if cfg != nil && cfg.BaseURL != "" {
				c, clientErr := flags.newClient()
				if clientErr != nil {
					report["api"] = fmt.Sprintf("client init error: %s", clientErr)
				} else {
					probeTimeout := flags.timeout
					if probeTimeout <= 0 || probeTimeout > 10*time.Second {
						probeTimeout = 10 * time.Second
					}
					probeCtx, cancel := context.WithTimeout(cmd.Context(), probeTimeout)
					defer cancel()
					// Step 1: Basic reachability via the configured transport.
					reachBody, reachErr := c.ProbeGetOnce(probeCtx, "/")
					var reachAPIErr *client.APIError
					switch {
					case reachErr == nil:
						// 2xx response — clearly reachable. Still inspect the
						// body for a known interstitial; some bot walls return
						// 200 with a JS challenge page.
						if vendor := looksLikeDoctorInterstitial(reachBody); vendor != "" {
							report["api"] = fmt.Sprintf("blocked by %s interstitial — the configured transport reached the wall. Try a different network, wait for the IP-level rate limit to clear, or check that the browser-chrome transport is bound correctly.", vendor)
						} else {
							report["api"] = "reachable"
						}
					case errors.As(reachErr, &reachAPIErr):
						// Non-2xx from the server. The network reached, the
						// server responded — that's "reachable" for our
						// purposes. Inspect the response body for a known
						// interstitial first; otherwise note the status.
						status := reachAPIErr.StatusCode
						if vendor := looksLikeDoctorInterstitial([]byte(reachAPIErr.Body)); vendor != "" {
							report["api"] = fmt.Sprintf("blocked by %s interstitial (HTTP %d) — the configured transport reached the wall.", vendor, status)
						} else {
							report["api"] = "reachable"
						}
					default:
						// Network-level failure: DNS, connection refused, TLS,
						// transport init, etc. The transport itself didn't
						// connect.
						report["api"] = fmt.Sprintf("unreachable: %s", reachErr)
					}

					// Step 2: Validate credentials with an authenticated probe.
					authHeader := cfg.AuthHeader()
					if authHeader == "" {
						// No auth configured — skip credential validation
					} else if reachErr != nil && !errors.As(reachErr, &reachAPIErr) {
						report["credentials"] = "skipped (API unreachable)"
					} else {
						_, authErr := c.ProbeGetOnce(probeCtx, "/models")
						var authAPIError *client.APIError
						switch {
						case authErr == nil:
							report["auth"] = "valid"
							report["credentials"] = "verified via GET /models"
						case errors.As(authErr, &authAPIError) && (authAPIError.StatusCode == 401 || authAPIError.StatusCode == 403):
							report["auth"] = "invalid"
							report["credentials"] = fmt.Sprintf("invalid (HTTP %d at /models)", authAPIError.StatusCode)
						default:
							report["credentials"] = fmt.Sprintf("present, not verified: %v", authErr)
						}
					}
				}
			} else if cfg != nil && cfg.BaseURL == "" {
				report["api"] = "not configured (set base_url in config file)"
			}
			// Local ledger: path, size, schema version and run count.
			report["cache"] = collectCacheReport(cmd.Context(), "")

			// Verify mode state. Surfaced so an operator who unintentionally
			// inherits the legacy verify-mode environment variable (parent shell, CI runner, container
			// image) detects the foot-gun without inspecting a response body.
			// Pairs with the synthetic envelope's verify_noop / reason literals
			// as a second diagnosis anchor.
			if cliutil.IsVerifyEnv() {
				if cliutil.IsVerifyLiveHTTPEnv() {
					report["verify_mode"] = "INFO ACTIVE — live HTTP opt-in (mutating verbs dial out)"
				} else {
					report["verify_mode"] = "INFO ACTIVE — mutating HTTP verbs short-circuit (legacy verify mode; no network calls for DELETE/POST/PUT/PATCH)"
				}
			} else {
				report["verify_mode"] = "normal operation"
			}

			report["version"] = version

			if flags.asJSON {
				if err := printJSONFiltered(cmd.OutOrStdout(), report, flags); err != nil {
					return err
				}
				return doctorExitForFailOn(failOn, report)
			}

			// Human-readable output with color
			w := cmd.OutOrStdout()
			checkKeys := []struct{ key, label string }{
				{"config", "Config"},
				{"auth", "Auth"},
				{"env_vars", "Env Vars"},
				{"verify_mode", "Verify Mode"},
				{"paths_warning", "Paths"},
				{"credentials_location_warning", "Credentials Storage"},
				{"api", "API"},
				{"credentials", "Credentials"},
			}
			for _, ck := range checkKeys {
				v, ok := report[ck.key]
				if !ok {
					continue
				}
				s := fmt.Sprintf("%v", v)
				indicator := green("OK")
				switch {
				case strings.HasPrefix(s, "WARN"):
					indicator = yellow("WARN")
				case strings.HasPrefix(s, "INFO"):
					indicator = yellow("INFO")
				case strings.HasPrefix(s, "ERROR"):
					indicator = red("FAIL")
				case strings.HasPrefix(s, "optional"):
					// Optional-auth CLI with no key set — informational, not a failure.
					indicator = yellow("INFO")
				case strings.Contains(s, "scope-limited"):
					indicator = yellow("WARN")
				case strings.Contains(s, "not verified"):
					// "present, not verified" — credentials are loaded but no
					// probe ran. Informational, not a warning; a clean config
					// shouldn't render yellow WARN in CI dashboards.
					indicator = yellow("INFO")
				case strings.Contains(s, "error") || strings.Contains(s, "not configured") || strings.Contains(s, "unreachable") || strings.Contains(s, "invalid") || strings.Contains(s, "missing"):
					indicator = red("FAIL")
				case s == "not required":
					// Public APIs: no auth needed is a healthy state, not a warning.
					indicator = green("OK")
				case strings.Contains(s, "not ") || strings.Contains(s, "skipped") || strings.Contains(s, "inferred"):
					indicator = yellow("WARN")
				}
				fmt.Fprintf(w, "  %s %s: %s\n", indicator, ck.label, s)
			}
			// Print info keys without status indicator
			for _, key := range []string{"config_path", "base_url", "auth_source", "credentials_location", "version"} {
				if v, ok := report[key]; ok {
					fmt.Fprintf(w, "  %s: %v\n", key, v)
				}
			}
			// Print auth setup hints (indented under Auth line)
			if hint, ok := report["auth_hint"]; ok {
				fmt.Fprintf(w, "  hint: %v\n", hint)
			}
			// Cache section: render after the primary health block so it
			// sits next to version info, mirroring the JSON report layout.
			if cacheAny, ok := report["cache"]; ok {
				if cacheRep, ok := cacheAny.(map[string]any); ok {
					renderCacheReport(w, cacheRep)
				}
			}
			if pathsAny, ok := report["paths"]; ok {
				if pathsRep, ok := pathsAny.(map[string]any); ok {
					renderPathsReport(w, pathsRep)
				}
			}
			return doctorExitForFailOn(failOn, report)
		},
	}
	cmd.Flags().StringVar(&failOn, "fail-on", "", "Exit non-zero for selected health gates. stale: cache freshness plus errors; warn: credential/path warnings plus errors; error: errors only. Default is never.")
	return cmd
}

func collectPathsReport() map[string]any {
	report := map[string]any{}
	resolutions, err := cliutil.AllPathResolutions()
	if err != nil {
		report["status"] = "error"
		report["detail"] = err.Error()
		return report
	}
	report["status"] = "ok"
	ignoredSeen := map[string]bool{}
	var ignored []map[string]string
	var notes []string
	for _, resolution := range resolutions {
		report[resolution.KindName] = map[string]any{
			"dir":    resolution.Dir,
			"rung":   resolution.Rung,
			"source": resolution.Source,
		}
		for _, skipped := range resolution.IgnoredOverrides {
			key := skipped.Name + "\x00" + skipped.Value
			if ignoredSeen[key] {
				continue
			}
			ignoredSeen[key] = true
			ignored = append(ignored, map[string]string{
				"name":  skipped.Name,
				"value": skipped.Value,
			})
		}
		if cliutil.HomeOverrideActive() && resolution.Rung == "per-kind-env" && (resolution.Kind == cliutil.PathKindData || resolution.Kind == cliutil.PathKindConfig) {
			notes = append(notes, fmt.Sprintf("--home shadowed for %s by %s", resolution.KindName, resolution.Source))
		}
	}
	if len(ignored) > 0 {
		report["skipped_relative_overrides"] = ignored
	}
	if len(notes) > 0 {
		report["notes"] = notes
	}
	return report
}

func pathsWarning(report map[string]any) string {
	if report == nil {
		return ""
	}
	var parts []string
	if raw, ok := report["skipped_relative_overrides"].([]map[string]string); ok && len(raw) > 0 {
		names := make([]string, 0, len(raw))
		for _, entry := range raw {
			names = append(names, entry["name"])
		}
		parts = append(parts, "relative override skipped: "+strings.Join(names, ", "))
	}
	if raw, ok := report["notes"].([]string); ok && len(raw) > 0 {
		parts = append(parts, "home override shadowed")
	}
	if len(parts) == 0 {
		return ""
	}
	return "WARN paths: " + strings.Join(parts, "; ")
}

func renderPathsReport(w io.Writer, rep map[string]any) {
	fmt.Fprintf(w, "  Paths:\n")
	for _, kind := range []string{"config", "data", "state", "cache"} {
		entry, ok := rep[kind].(map[string]any)
		if !ok {
			continue
		}
		fmt.Fprintf(w, "    %s: %v (%v)\n", kind, entry["dir"], entry["source"])
	}
	if raw, ok := rep["skipped_relative_overrides"].([]map[string]string); ok && len(raw) > 0 {
		fmt.Fprintf(w, "    skipped_relative_overrides:\n")
		for _, entry := range raw {
			fmt.Fprintf(w, "      %s=%q\n", entry["name"], entry["value"])
		}
	}
	if raw, ok := rep["notes"].([]string); ok && len(raw) > 0 {
		fmt.Fprintf(w, "    notes:\n")
		for _, note := range raw {
			fmt.Fprintf(w, "      %s\n", note)
		}
	}
}
func collectCredentialsLocationReport(report map[string]any, cfg *config.Config) {
	if cfg == nil {
		return
	}
	if cfg.CredentialSource != "" {
		report["credentials_location"] = cfg.CredentialSource
	} else {
		report["credentials_location"] = "none"
	}
	locations := []string{}
	credsPresent, err := cliutil.CredentialsFileHasValues()
	if err == nil && credsPresent {
		locations = append(locations, "credentials file")
	}
	legacySecretsElsewhere := ""
	for _, path := range legacyCredentialProbePaths(cfg) {
		ok, err := config.FileHasCredentialFields(path)
		if err == nil && ok {
			locations = append(locations, path)
			if path != cfg.Path {
				legacySecretsElsewhere = path
			}
		}
	}
	if len(locations) > 0 {
		report["credentials_locations"] = locations
	}
	if credsPresent && len(locations) > 1 {
		if legacySecretsElsewhere != "" {
			report["credentials_location_warning"] = "WARN credentials stored in more than one location; legacy secrets remain at " + legacySecretsElsewhere + "; run auth set-token or auth logout to consolidate and remove legacy secrets"
		} else {
			report["credentials_location_warning"] = "WARN credentials stored in more than one location; current reads use credentials file; run auth set-token or auth logout to consolidate"
		}
	}
}

func legacyCredentialProbePaths(cfg *config.Config) []string {
	seen := map[string]bool{}
	var paths []string
	add := func(path string) {
		if path == "" || seen[path] {
			return
		}
		seen[path] = true
		paths = append(paths, path)
	}
	if cfg != nil && cfg.Path != "" {
		// Probe only the active config; a same-dir standard-named file may
		// belong to an unrelated CLI sharing that directory.
		add(cfg.Path)
	}
	if legacyPath, err := config.LegacyConfigPath(); err == nil {
		add(legacyPath)
	}
	return paths
}

// doctorExitForFailOn returns a non-nil error when the report's worst
// status meets the --fail-on gate. "error" trips on failing sections, "warn"
// trips on deliberate WARN sections plus errors, and "stale" trips on cache
// freshness plus errors. The default empty string means never fail on status.
func doctorExitForFailOn(failOn string, report map[string]any) error {
	if failOn == "" {
		return nil
	}
	worstError := false
	worstWarn := false
	worstStale := false
	for _, v := range report {
		s, ok := v.(string)
		if ok {
			if strings.Contains(s, "error") || strings.Contains(s, "unreachable") || strings.Contains(s, "invalid") || strings.Contains(s, "missing") {
				worstError = true
			}
			if strings.HasPrefix(s, "WARN") {
				worstWarn = true
			}
		}
		if m, ok := v.(map[string]any); ok {
			if st, _ := m["status"].(string); st == "error" {
				worstError = true
			} else if st == "warn" {
				worstWarn = true
			} else if st == "stale" {
				worstStale = true
			}
		}
	}
	switch failOn {
	case "error":
		if worstError {
			return fmt.Errorf("doctor: --fail-on=error triggered")
		}
	case "warn":
		if worstError || worstWarn {
			return fmt.Errorf("doctor: --fail-on=warn triggered")
		}
	case "stale":
		if worstError || worstStale {
			return fmt.Errorf("doctor: --fail-on=stale triggered")
		}
	default:
		return fmt.Errorf("doctor: unknown --fail-on value %q (valid: stale, warn, error)", failOn)
	}
	return nil
}

// collectCacheReport summarises the local run ledger: where it lives, its
// size and schema, and how many runs it holds. Never panics on a missing or
// unreadable database; it reports status=unknown or status=error instead.
func collectCacheReport(ctx context.Context, _ string) map[string]any {
	report := map[string]any{}
	dbPath := defaultDBPath("mixlayer")
	report["db_path"] = dbPath

	fi, err := os.Stat(dbPath)
	if err != nil {
		if os.IsNotExist(err) {
			report["status"] = "unknown"
			report["hint"] = "Local ledger not created yet; it is created on the first ask, ladder or shield run."
			return report
		}
		report["status"] = "error"
		report["error"] = err.Error()
		return report
	}
	report["db_bytes"] = fi.Size()

	s, err := store.OpenReadOnlyContext(ctx, dbPath)
	if err != nil {
		report["status"] = "error"
		report["error"] = err.Error()
		return report
	}
	defer s.Close()

	if v, verr := s.SchemaVersion(); verr == nil {
		report["schema_version"] = v
	}
	var runs int64
	var spend sql.NullFloat64
	if err := s.DB().QueryRowContext(ctx, `SELECT count(*), sum(cost_usd) FROM runs`).Scan(&runs, &spend); err == nil {
		report["runs"] = runs
		if spend.Valid {
			report["recorded_cost_usd"] = spend.Float64
		}
	}
	report["status"] = "ok"
	return report
}

func renderCacheReport(w io.Writer, rep map[string]any) {
	status, _ := rep["status"].(string)
	indicator := green("OK")
	switch status {
	case "stale":
		indicator = yellow("WARN")
	case "error":
		indicator = red("FAIL")
	case "unknown":
		indicator = yellow("INFO")
	}
	fmt.Fprintf(w, "  %s Ledger: %s\n", indicator, status)
	if v, ok := rep["db_path"]; ok {
		fmt.Fprintf(w, "    db_path: %v\n", v)
	}
	if v, ok := rep["schema_version"]; ok {
		fmt.Fprintf(w, "    schema_version: %v\n", v)
	}
	if v, ok := rep["db_bytes"]; ok {
		fmt.Fprintf(w, "    db_bytes: %v\n", v)
	}
	if v, ok := rep["runs"]; ok {
		fmt.Fprintf(w, "    runs: %v\n", v)
	}
	if v, ok := rep["recorded_cost_usd"]; ok {
		fmt.Fprintf(w, "    recorded_cost_usd: %v\n", v)
	}
	if hint, ok := rep["hint"]; ok {
		fmt.Fprintf(w, "    hint: %v\n", hint)
	}
}
