// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

package cliutil

import "os"

// VerifyEnvVar puts the CLI in mock mode for automated test runs. Commands
// with visible side effects (opening browser tabs, sending notifications,
// dialing OS handlers) short-circuit when it is "1".
//
// The transport layer in internal/client also gates mutating HTTP verbs
// (DELETE/POST/PUT/PATCH) on this variable: in mock mode those requests
// return a synthetic envelope and never dial. Test harnesses that run
// against a local fake server opt back in with VerifyLiveHTTPEnvVar.
const VerifyEnvVar = "MIXLAYER_VERIFY"

// VerifyLiveHTTPEnvVar opts a mock-mode process back in to the real HTTP
// path for mutating verbs. It only has an effect when VerifyEnvVar is also
// set; on its own it changes nothing.
const VerifyLiveHTTPEnvVar = "MIXLAYER_VERIFY_LIVE_HTTP"

// SmokeEnvVar marks a bounded smoke run against the real API. Long-running
// commands (full sync loops, bulk walks) use it to limit work, for example
// by fetching a single page. It never substitutes mock data for real calls.
const SmokeEnvVar = "MIXLAYER_SMOKE"

// IsVerifyEnv reports whether the process is running in mock mode.
//
//	if cliutil.IsVerifyEnv() {
//	    fmt.Fprintln(cmd.OutOrStdout(), "would launch:", url)
//	    return nil
//	}
func IsVerifyEnv() bool {
	return os.Getenv(VerifyEnvVar) == "1"
}

// IsVerifyLiveHTTPEnv reports whether a mock-mode process has opted back in
// to real HTTP for mutating verbs. Only meaningful when IsVerifyEnv is true.
//
//	if !readOnlyIntent && isMutatingVerb(method) && cliutil.IsVerifyEnv() && !cliutil.IsVerifyLiveHTTPEnv() {
//	    // synthetic envelope, no network call
//	}
//
// readOnlyIntent is set by Client.doRead() callers: POST-based reads such as
// queries and searches that do not change remote state.
func IsVerifyLiveHTTPEnv() bool {
	return os.Getenv(VerifyLiveHTTPEnvVar) == "1"
}

// IsSmokeEnv reports whether the process is a bounded smoke run.
func IsSmokeEnv() bool {
	return os.Getenv(SmokeEnvVar) == "1"
}

// JiniEnvVar turns on discovery of the experimental Jini decision commands
// (decide, decide batch, escalate --checker jini). With it unset the commands
// still run, but they are left out of help, which, agent-context, MCP and the
// embedded skill until the decision API is publicly documented.
const JiniEnvVar = "MIXLAYER_ENABLE_JINI"

// JiniEnabled reports whether experimental Jini commands are advertised.
func JiniEnabled() bool {
	return os.Getenv(JiniEnvVar) == "1"
}
