// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

package shield

import (
	"context"
	"encoding/json"
	"fmt"
	"regexp"
	"sort"
	"strings"
	"unicode"
	"unicode/utf8"

	"github.com/mixlayer/cli/internal/store"
)

type ModelCall func(context.Context, string) (string, error)

var modelRisk = map[string]int{
	"PERSON": 8, "ADDRESS": 8, "ORG": 4, "LOCATION": 5,
	"DATE_OF_BIRTH": 8, "ID_NUMBER": 10, "MEDICAL": 9, "ACCOUNT": 9, "OTHER": 5,
	"EMAIL": 9, "PHONE": 7, "SSN": 10, "CARD": 10, "IP": 5, "URL": 4,
}

func ValidKind(kind string) bool {
	if _, ok := modelRisk[kind]; ok {
		return true
	}
	switch kind {
	case "EMAIL", "PHONE", "SSN", "IP", "URL", "CARD":
		return true
	}
	return false
}

func KindRisk(kind string) int {
	if risk, ok := modelRisk[kind]; ok {
		return risk
	}
	for _, d := range detectors {
		if d.kind == kind {
			return d.risk
		}
	}
	if kind == "CARD" {
		return 10
	}
	return 5
}

func MergeEntities(in []Entity) []Entity {
	sort.Slice(in, func(i, j int) bool {
		if in[i].Start == in[j].Start {
			return in[i].End > in[j].End
		}
		return in[i].Start < in[j].Start
	})
	return dedupeOverlaps(in)
}

func DetectWith(ctx context.Context, text string, dictionary []store.DictionaryTerm, call ModelCall, fallback bool) ([]Entity, bool, error) {
	entities, usedFallback, _, err := DetectWithStats(ctx, text, dictionary, call, fallback)
	return entities, usedFallback, err
}

func DetectWithStats(ctx context.Context, text string, dictionary []store.DictionaryTerm, call ModelCall, fallback bool) ([]Entity, bool, int, error) {
	regex := Detect(text)
	entities := append([]Entity(nil), regex...)
	usedFallback := false
	ignored := 0
	if call != nil {
		for _, chunk := range chunks(text) {
			prompt := `Return ONLY strict JSON of this shape: {"entities":[{"kind":"PERSON|ADDRESS|ORG|LOCATION|DATE_OF_BIRTH|ID_NUMBER|MEDICAL|ACCOUNT|OTHER","text":"<exact substring>"}]}. Identify sensitive spans. Copy each text verbatim. Do not redact, paraphrase, or invent. Text: ` + string(mustJSON(chunk))
			answer, err := call(ctx, prompt)
			if err == nil {
				var response struct {
					Entities []struct {
						Kind string `json:"kind"`
						Text string `json:"text"`
					} `json:"entities"`
				}
				if json.Unmarshal([]byte(answer), &response) != nil || response.Entities == nil {
					err = fmt.Errorf("model detector returned invalid JSON entities")
				} else {
					for _, item := range response.Entities {
						if strings.TrimSpace(item.Text) == "" {
							ignored++
							continue
						}
						// Models label kinds freely ("EMAIL_ADDRESS", "NAME"). An
						// unknown label still marks private text, so mask it as
						// OTHER rather than discarding every finding.
						kind := strings.ToUpper(strings.TrimSpace(item.Kind))
						risk, ok := modelRisk[kind]
						if !ok {
							kind, risk = "OTHER", modelRisk["OTHER"]
						}
						matched := false
						for offset := 0; offset < len(text); {
							idx := strings.Index(text[offset:], item.Text)
							if idx < 0 {
								break
							}
							matched = true
							start := offset + idx
							entities = append(entities, Entity{Kind: kind, Value: item.Text, Start: start, End: start + len(item.Text), Risk: risk})
							offset = start + len(item.Text)
						}
						if !matched {
							ignored++
						}
					}
				}
			}
			if err != nil {
				if !fallback {
					return nil, false, ignored, fmt.Errorf("model detector failed: %w", err)
				}
				entities = append([]Entity(nil), regex...)
				usedFallback = true
				break
			}
		}
	}
	entities = coverOverlaps(text, entities)
	// Dictionary matches take precedence over overlapping heuristic or model spans.
	dictEntities := DictionaryMatches(text, dictionary)
	for _, d := range dictEntities {
		for i := 0; i < len(entities); {
			e := entities[i]
			if e.Start < d.End && d.Start < e.End {
				d.Start = min(d.Start, e.Start)
				d.End = max(d.End, e.End)
				d.Value = text[d.Start:d.End]
				d.Risk = max(d.Risk, e.Risk)
				entities = append(entities[:i], entities[i+1:]...)
				i = 0 // An expanded span may now overlap an earlier entity.
				continue
			}
			i++
		}
		entities = append(entities, d)
	}
	return MergeEntities(entities), usedFallback, ignored, nil
}

func coverOverlaps(text string, in []Entity) []Entity {
	if len(in) == 0 {
		return nil
	}
	sort.Slice(in, func(i, j int) bool {
		if in[i].Start == in[j].Start {
			return in[i].End > in[j].End
		}
		return in[i].Start < in[j].Start
	})
	var out []Entity
	start, end, best := in[0].Start, in[0].End, in[0]
	flush := func() { best.Start = start; best.End = end; best.Value = text[start:end]; out = append(out, best) }
	for _, e := range in[1:] {
		if e.Start < end {
			end = max(end, e.End)
			if betterEntity(e, best) {
				best = e
			}
			continue
		}
		flush()
		start, end, best = e.Start, e.End, e
	}
	flush()
	return out
}

func DictionaryMatches(text string, terms []store.DictionaryTerm) []Entity {
	var out []Entity
	for _, term := range terms {
		if term.Term == "" {
			continue
		}
		re := regexp.MustCompile(`(?i)` + regexp.QuoteMeta(term.Term))
		for _, m := range re.FindAllStringIndex(text, -1) {
			if m[0] > 0 {
				r, _ := utf8.DecodeLastRuneInString(text[:m[0]])
				if unicode.IsLetter(r) || unicode.IsDigit(r) || r == '_' {
					continue
				}
			}
			if m[1] < len(text) {
				r, _ := utf8.DecodeRuneInString(text[m[1]:])
				if unicode.IsLetter(r) || unicode.IsDigit(r) || r == '_' {
					continue
				}
			}
			out = append(out, Entity{Kind: term.Kind, Value: text[m[0]:m[1]], Start: m[0], End: m[1], Risk: KindRisk(term.Kind)})
		}
	}
	sort.Slice(out, func(i, j int) bool { return out[i].Start < out[j].Start })
	merged := make([]Entity, 0, len(out))
	for _, e := range out {
		if len(merged) > 0 && e.Start < merged[len(merged)-1].End {
			last := &merged[len(merged)-1]
			last.End = max(last.End, e.End)
			last.Value = text[last.Start:last.End]
			last.Risk = max(last.Risk, e.Risk)
		} else {
			merged = append(merged, e)
		}
	}
	return merged
}

func mustJSON(s string) []byte { b, _ := json.Marshal(s); return b }

func chunks(text string) []string {
	if text == "" {
		return nil
	}
	const size, overlap = 6000, 200
	runes := []rune(text)
	var out []string
	for start := 0; start < len(runes); {
		end := start + size
		if end >= len(runes) {
			out = append(out, string(runes[start:]))
			break
		}
		boundary := end
		for i := end - 1; i > start+size/2; i-- {
			if runes[i] == '\n' || runes[i] == '.' || runes[i] == '!' || runes[i] == '?' {
				boundary = i + 1
				break
			}
		}
		end = boundary
		out = append(out, string(runes[start:end]))
		start = end - overlap
	}
	return out
}
