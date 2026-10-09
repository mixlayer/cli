// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

package shield

import (
	"context"
	"regexp"
	"strings"

	"github.com/mixlayer/cli/internal/store"
)

type RestoreReport struct {
	Text             string   `json:"text"`
	RestoredCount    int      `json:"restored_count"`
	UnrestoredCount  int      `json:"unrestored_count"`
	UnrestoredTokens []string `json:"unrestored_tokens"`
	Repairs          []string `json:"repairs"`
}

var placeholder = regexp.MustCompile(`\b[A-Za-z]+(?:_[A-Za-z]+)*\s*_\s*[0-9a-fA-F](?:\s*[0-9a-fA-F]){7,15}\b`)
var spaces = regexp.MustCompile(`\s+`)

func RehydrateChecked(ctx context.Context, s *store.Store, answer string) (RestoreReport, error) {
	return rehydrateChecked(ctx, s, answer, nil)
}

// RehydrateCheckedForTokens restores only placeholders sent in this request.
func RehydrateCheckedForTokens(ctx context.Context, s *store.Store, answer string, allowed []string) (RestoreReport, error) {
	allow := map[string]bool{}
	for _, token := range allowed {
		allow[token] = true
	}
	return rehydrateChecked(ctx, s, answer, allow)
}

func rehydrateChecked(ctx context.Context, s *store.Store, answer string, allow map[string]bool) (RestoreReport, error) {
	allMapping, err := s.VaultTokenMap(ctx)
	if err != nil {
		return RestoreReport{}, err
	}
	mapping := map[string]string{}
	folded := map[string][]string{}
	// bySuffix indexes this request's placeholders by their hex ID, so a
	// placeholder the model relabelled (COMPANY_ab12cd34 written back as
	// CUSTOMER_ab12cd34) still restores when exactly one sent token has that
	// ID. Only used for request-scoped restores.
	bySuffix := map[string][]string{}
	for token, value := range allMapping {
		folded[strings.ToUpper(token)] = append(folded[strings.ToUpper(token)], token)
		if allow == nil || allow[token] {
			mapping[token] = value
		}
		if allow != nil && allow[token] {
			if sfx := tokenSuffix(token); sfx != "" {
				bySuffix[sfx] = append(bySuffix[sfx], token)
			}
		}
	}
	result := RestoreReport{UnrestoredTokens: []string{}, Repairs: []string{}}
	seen := map[string]bool{}
	var out strings.Builder
	prev := 0
	for _, m := range placeholder.FindAllStringIndex(answer, -1) {
		candidate := answer[m[0]:m[1]]
		out.WriteString(answer[prev:m[0]])
		if value, ok := mapping[candidate]; ok {
			out.WriteString(value)
			result.RestoredCount++
		} else {
			normalized := strings.ToUpper(spaces.ReplaceAllString(candidate, ""))
			matches := folded[normalized]
			value, ok := "", false
			if len(matches) == 1 {
				value, ok = mapping[matches[0]]
			}
			// A candidate that names some other real vault token is never
			// remapped by ID; it may belong to a different request.
			if !ok && len(folded[normalized]) == 0 {
				if same := bySuffix[tokenSuffix(normalized)]; len(same) == 1 {
					matches = same
					value, ok = mapping[same[0]]
				}
			}
			if ok {
				out.WriteString(value)
				result.RestoredCount++
				result.Repairs = append(result.Repairs, candidate+" -> "+matches[0])
			} else {
				out.WriteString(candidate)
				result.UnrestoredCount++
				if !seen[candidate] {
					result.UnrestoredTokens = append(result.UnrestoredTokens, candidate)
					seen[candidate] = true
				}
			}
		}
		prev = m[1]
	}
	out.WriteString(answer[prev:])
	result.Text = out.String()
	return result, nil
}

// tokenSuffix returns the lowercase hex ID after the last underscore, or ""
// when the token has no ID of at least eight hex digits.
func tokenSuffix(token string) string {
	i := strings.LastIndex(token, "_")
	if i < 0 {
		return ""
	}
	sfx := strings.ToLower(spaces.ReplaceAllString(token[i+1:], ""))
	if len(sfx) < 8 {
		return ""
	}
	for _, r := range sfx {
		if (r < '0' || r > '9') && (r < 'a' || r > 'f') {
			return ""
		}
	}
	return sfx
}
