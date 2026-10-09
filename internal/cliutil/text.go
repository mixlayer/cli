// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

package cliutil

import (
	"html"
	"regexp"
	"strings"
	"time"
)

// CleanText normalizes scraped text by trimming whitespace and decoding
// HTML entities. Always use this when extracting strings from HTML or
// JSON-LD; otherwise entities such as "&#39;" leak into output.
//
// Single unescape pass: "&amp;amp;" -> "&amp;" (matches html.UnescapeString
// stdlib behavior). If you need multiple passes you almost always have a
// deeper escaping problem upstream — fix there, not here.
func CleanText(s string) string {
	return html.UnescapeString(strings.TrimSpace(s))
}

// ParseStoredTime parses timestamps read back from the SQLite store.
// modernc.org/sqlite can serialize time.Time using Go's native
// time.String format, while other writers store RFC3339.
// Use this helper instead of a single time.Parse(time.RFC3339, value) call
// when scanning timestamp columns from the store.
func ParseStoredTime(s string) time.Time {
	s = strings.TrimSpace(s)
	if s == "" {
		return time.Time{}
	}
	for _, layout := range []string{
		time.RFC3339Nano,
		time.RFC3339,
		"2006-01-02 15:04:05.999999999 -0700 MST",
		"2006-01-02 15:04:05.999999 -0700 MST",
		"2006-01-02 15:04:05.999 -0700 MST",
		"2006-01-02 15:04:05 -0700 MST",
		"2006-01-02 15:04:05.999999999 -0700",
		"2006-01-02 15:04:05 -0700",
	} {
		if t, err := time.Parse(layout, s); err == nil {
			return t
		}
	}
	return time.Time{}
}

// LooksLikeAuthError checks if an error message body contains auth-related keywords.
func LooksLikeAuthError(msg string) bool {
	lower := strings.ToLower(msg)
	patterns := []string{
		`\bkey\b`,
		`\btoken\b`,
		`\bunauthorized\b`,
		`\bapi_key\b`,
		`missing.{0,20}key`,
		`required.{0,20}key`,
		`\bforbidden\b`,
		`\bauthenticat`,
		`\bcredential`,
	}
	for _, p := range patterns {
		if matched, _ := regexp.MatchString(p, lower); matched {
			return true
		}
	}
	return false
}

// SanitizeErrorBody truncates and strips credential-shaped strings from error output.
func SanitizeErrorBody(msg string) string {
	if len(msg) > 200 {
		msg = msg[:200] + "..."
	}
	credPatterns := regexp.MustCompile(`(?i)(sk-[a-zA-Z0-9]{8,}|sk_live_[a-zA-Z0-9]+|Bearer\s+[a-zA-Z0-9._\-]+|(?:key|token)=[a-zA-Z0-9._\-]+)`)
	msg = credPatterns.ReplaceAllString(msg, "[REDACTED]")
	return msg
}
