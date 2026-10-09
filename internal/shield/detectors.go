// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

package shield

import (
	"regexp"
	"sort"
	"strings"
	"unicode"
	"unicode/utf8"
)

type Entity struct {
	Kind  string `json:"kind"`
	Value string `json:"value"`
	Start int    `json:"start"`
	End   int    `json:"end"`
	Risk  int    `json:"risk"`
}

var detectors = []struct {
	kind string
	risk int
	re   *regexp.Regexp
}{
	{"EMAIL", 9, regexp.MustCompile(`(?i)\b[A-Z0-9._%+\-]+@[A-Z0-9.\-]+\.[A-Z]{2,}\b`)},
	{"EMAIL", 8, regexp.MustCompile(`(?i)\b[A-Z0-9_%+\-]+(?:\s+dot\s+[A-Z0-9_%+\-]+)*\s+(?:at|\[at\]|\(at\))\s+[A-Z0-9\-]+(?:\s+dot\s+[A-Z0-9\-]+)+\b`)},
	{"EMAIL", 8, regexp.MustCompile(`(?i)\b[A-Z0-9._%+\-]+\s*(?:\[at\]|\(at\))\s*[A-Z0-9.\-]+\.[A-Z]{2,}\b`)},
	{"PHONE", 7, regexp.MustCompile(`\b(?:\+?1[\s.\-]?)?(?:\(?\d{3}\)?[\s.\-]?)\d{3}[\s.\-]?\d{4}\b`)},
	{"PHONE", 7, regexp.MustCompile(`\+\d{1,3}(?:[\s().\-]*\d){7,12}\b`)},
	{"PHONE", 7, regexp.MustCompile(`\b0\d{2,4}[\s().\-]*\d{3,4}[\s().\-]*\d{3,4}\b`)},
	{"SSN", 10, regexp.MustCompile(`\b\d{3}-\d{2}-\d{4}\b`)},
	{"ID_NUMBER", 9, regexp.MustCompile(`(?i)\b[A-Z]{2}\s*\d{2}\s*\d{2}\s*\d{2}\s*[A-D]\b`)},
	{"ID_NUMBER", 8, regexp.MustCompile(`\b\d{3}[ -]\d{3}[ -]\d{2,3}\b`)},
	{"IBAN_CANDIDATE", 9, regexp.MustCompile(`(?i)\b[A-Z]{2}\d{2}(?:\s?[A-Z0-9]{4}){2,7}(?:\s?[A-Z0-9]{1,4})?\b|\b[A-Z]{2}\d{2}[A-Z0-9]{11,30}\b`)},
	{"IP", 5, regexp.MustCompile(`\b(?:\d{1,3}\.){3}\d{1,3}\b`)},
	{"URL", 4, regexp.MustCompile(`\bhttps?://[^\s<>"']+`)},
	{"CARD_CANDIDATE", 10, regexp.MustCompile(`\b(?:\d[ -]*?){13,19}\b`)},
}

func Detect(text string) []Entity {
	var out []Entity
	for _, d := range detectors {
		matches := d.re.FindAllStringIndex(text, -1)
		for _, m := range matches {
			value := text[m[0]:m[1]]
			kind := d.kind
			if kind == "CARD_CANDIDATE" {
				if !validLuhn(value) {
					continue
				}
				kind = "CARD"
			}
			risk := d.risk
			if kind == "IBAN_CANDIDATE" {
				kind = "ACCOUNT"
				if !validIBAN(value) {
					risk = 6
				}
			}
			if kind == "IP" && !validIPv4(value) {
				continue
			}
			out = append(out, Entity{Kind: kind, Value: value, Start: m[0], End: m[1], Risk: risk})
		}
	}
	out = append(out, detectLikelyNames(text)...)
	sort.Slice(out, func(i, j int) bool {
		if out[i].Start == out[j].Start {
			return out[i].End > out[j].End
		}
		return out[i].Start < out[j].Start
	})
	return dedupeOverlaps(out)
}

func validIBAN(value string) bool {
	s := strings.ToUpper(strings.ReplaceAll(value, " ", ""))
	if len(s) < 15 || len(s) > 34 {
		return false
	}
	rearranged := s[4:] + s[:4]
	mod := 0
	for _, r := range rearranged {
		switch {
		case r >= '0' && r <= '9':
			mod = (mod*10 + int(r-'0')) % 97
		case r >= 'A' && r <= 'Z':
			n := int(r-'A') + 10
			mod = (mod*100 + n) % 97
		default:
			return false
		}
	}
	return mod == 1
}

func RiskScore(entities []Entity) int {
	maxRisk := 0
	for _, e := range entities {
		if e.Risk > maxRisk {
			maxRisk = e.Risk
		}
	}
	return maxRisk
}

func dedupeOverlaps(in []Entity) []Entity {
	var out []Entity
	for i := 0; i < len(in); {
		best := in[i]
		clusterEnd := in[i].End
		j := i + 1
		for j < len(in) && in[j].Start < clusterEnd {
			if in[j].End > clusterEnd {
				clusterEnd = in[j].End
			}
			if betterEntity(in[j], best) {
				best = in[j]
			}
			j++
		}
		out = append(out, best)
		i = j
	}
	sort.Slice(out, func(i, j int) bool { return out[i].Start < out[j].Start })
	return out
}

func betterEntity(candidate, current Entity) bool {
	if candidate.Kind == "ACCOUNT" && (current.Kind == "CARD" || current.Kind == "PHONE") && candidate.Start <= current.Start && candidate.End >= current.End {
		return true
	}
	if current.Kind == "ACCOUNT" && (candidate.Kind == "CARD" || candidate.Kind == "PHONE") && current.Start <= candidate.Start && current.End >= candidate.End {
		return false
	}
	if candidate.Risk != current.Risk {
		return candidate.Risk > current.Risk
	}
	candidateLen := candidate.End - candidate.Start
	currentLen := current.End - current.Start
	if candidateLen != currentLen {
		return candidateLen > currentLen
	}
	return candidate.Start < current.Start
}

func validLuhn(s string) bool {
	var digits []int
	for _, r := range s {
		if unicode.IsDigit(r) {
			digits = append(digits, int(r-'0'))
		}
	}
	if len(digits) < 13 {
		return false
	}
	sum := 0
	double := false
	for i := len(digits) - 1; i >= 0; i-- {
		n := digits[i]
		if double {
			n *= 2
			if n > 9 {
				n -= 9
			}
		}
		sum += n
		double = !double
	}
	return sum%10 == 0
}

func validIPv4(s string) bool {
	parts := strings.Split(s, ".")
	if len(parts) != 4 {
		return false
	}
	for _, p := range parts {
		if p == "" || len(p) > 3 {
			return false
		}
		n := 0
		for _, r := range p {
			if !unicode.IsDigit(r) {
				return false
			}
			n = n*10 + int(r-'0')
		}
		if n > 255 {
			return false
		}
	}
	return true
}

// detectLikelyNames finds runs of capitalized words that look like personal
// names. Leading command or greeting words ("Contact", "Dear", "Please call")
// are trimmed so the whole name is masked rather than "Contact Jordan" with
// the surname left in the clear. A single capitalized word counts as a name
// only when a word right before it signals a person ("Dear Pebble", "ask Art",
// "belongs to Pebble").
func detectLikelyNames(text string) []Entity {
	var out []Entity
	for _, m := range capitalizedRunRe.FindAllStringIndex(text, -1) {
		toks := tokenSpans(text, m[0], m[1])
		personCue := false
		for len(toks) > 0 {
			w := text[toks[0][0]:toks[0][1]]
			if _, ok := personCueWords[strings.ToLower(w)]; ok {
				personCue = true
			} else if _, ok := leadingNonNameWords[w]; !ok {
				break
			}
			toks = toks[1:]
		}
		for len(toks) > 0 {
			w := text[toks[len(toks)-1][0]:toks[len(toks)-1][1]]
			if _, ok := nonPersonToken[w]; !ok {
				break
			}
			toks = toks[:len(toks)-1]
		}
		if len(toks) == 0 {
			continue
		}
		start, end := toks[0][0], toks[len(toks)-1][1]
		value := text[start:end]
		if len(toks) == 1 {
			if !personCue && !personCueBefore(text[:start]) {
				continue
			}
			if _, ok := leadingNonNameWords[value]; ok {
				continue
			}
		}
		if shouldSkipLikelyName(text, start, value) {
			continue
		}
		out = append(out, Entity{Kind: "PERSON", Value: value, Start: start, End: end, Risk: 6})
	}
	return out
}

// capitalizedRunRe matches one to five capitalized words (with optional
// hyphen or apostrophe parts such as O'Neil or Smith-Jones) separated by
// whitespace, including a line break inside a name.
var capitalizedRunRe = regexp.MustCompile(`\b[A-Z][a-z]+(?:['\-][A-Z]?[a-z]+)?(?:[ \t]*\n?[ \t]*[A-Z][a-z]+(?:['\-][A-Z]?[a-z]+)?){0,4}\b`)

func tokenSpans(text string, start, end int) [][2]int {
	var spans [][2]int
	i := start
	for i < end {
		for i < end && (text[i] == ' ' || text[i] == '\t' || text[i] == '\n' || text[i] == '\r') {
			i++
		}
		j := i
		for j < end && text[j] != ' ' && text[j] != '\t' && text[j] != '\n' && text[j] != '\r' {
			j++
		}
		if j > i {
			spans = append(spans, [2]int{i, j})
		}
		i = j
	}
	return spans
}

// personCueWords, when they open a capitalized run, signal that the next
// word is a person ("Contact Zora", "Dear Pebble").
var personCueWords = map[string]struct{}{
	"ask": {}, "call": {}, "contact": {}, "dear": {}, "email": {}, "hello": {}, "hey": {},
	"hi": {}, "message": {}, "nickname": {}, "alias": {}, "ping": {}, "tell": {}, "text": {},
	"thanks": {}, "thank": {}, "cc": {}, "attn": {},
}

// leadingNonNameWords are capitalized sentence openers that are not names.
var leadingNonNameWords = map[string]struct{}{
	"The": {}, "This": {}, "That": {}, "These": {}, "Those": {}, "Please": {}, "Our": {}, "Your": {},
	"My": {}, "His": {}, "Her": {}, "Their": {}, "When": {}, "Then": {}, "Today": {}, "Tomorrow": {},
	"Yesterday": {}, "If": {}, "And": {}, "But": {}, "Or": {}, "So": {}, "Also": {}, "Note": {},
	"Mixed": {}, "Fictional": {}, "Mock": {}, "Send": {}, "Meet": {}, "Callback": {}, "Customer": {},
	"Patient": {}, "User": {}, "Owner": {}, "Name": {}, "From": {}, "To": {}, "Subject": {}, "Re": {},
	"Monday": {}, "Tuesday": {}, "Wednesday": {}, "Thursday": {}, "Friday": {}, "Saturday": {}, "Sunday": {},
	"January": {}, "February": {}, "March": {}, "April": {}, "June": {}, "July": {}, "August": {},
	"September": {}, "October": {}, "November": {}, "December": {}, "Die": {}, "La": {}, "El": {}, "Le": {},
}

// personCueBefore reports whether the words right before a single
// capitalized word point at a person.
func personCueBefore(prefix string) bool {
	w1 := strings.ToLower(wordBefore(prefix))
	if w1 == "" {
		return false
	}
	if _, ok := personCueWords[w1]; ok {
		return true
	}
	switch w1 {
	case "say", "says", "signed", "named", "called", "by", "with", "dr", "mr", "mrs", "ms":
		return true
	case "to", "for":
		rest := strings.TrimRightFunc(prefix, func(r rune) bool { return !unicode.IsLetter(r) })
		rest = rest[:len(rest)-len(wordBefore(rest))]
		w2 := strings.ToLower(wordBefore(rest))
		return w2 == "belongs" || w2 == "belong" || w2 == "talk" || w2 == "spoke" || w2 == "assigned" || w2 == "sent"
	}
	return false
}

var nonPersonPhrase = map[string]struct{}{
	"Amazon Web":    {},
	"Google Cloud":  {},
	"Los Angeles":   {},
	"Mixlayer Labs": {},
	"New York":      {},
	"San Francisco": {},
}

var nonPersonToken = map[string]struct{}{
	"Avenue": {}, "Ave": {}, "Boulevard": {}, "Blvd": {}, "City": {}, "Cloud": {}, "County": {},
	"Court": {}, "Drive": {}, "Inc": {}, "Labs": {}, "Lane": {}, "Limited": {}, "Ltd": {},
	"Mixlayer": {}, "Platform": {}, "Road": {}, "Services": {}, "Street": {}, "Systems": {},
	"University": {}, "Way": {}, "York": {},
	"Analytics": {}, "Board": {}, "Center": {}, "Committee": {}, "Department": {}, "Engineering": {},
	"Group": {}, "Manager": {}, "Office": {}, "Report": {}, "Request": {}, "Review": {}, "Square": {},
	"Support": {}, "Team": {}, "Project": {}, "Program": {}, "Plan": {}, "Policy": {},
}

var locationPrepositions = map[string]struct{}{
	"at":   {},
	"in":   {},
	"near": {},
}

func shouldSkipLikelyName(text string, start int, value string) bool {
	if _, ok := nonPersonPhrase[value]; ok {
		return true
	}
	for _, part := range strings.Fields(value) {
		if _, ok := nonPersonToken[part]; ok {
			return true
		}
	}
	if prevWord := wordBefore(text[:start]); prevWord != "" {
		if _, ok := locationPrepositions[strings.ToLower(prevWord)]; ok {
			return true
		}
	}
	return false
}

func wordBefore(prefix string) string {
	prefix = strings.TrimRightFunc(prefix, func(r rune) bool {
		return !unicode.IsLetter(r)
	})
	if prefix == "" {
		return ""
	}
	start := len(prefix)
	for start > 0 {
		r, size := utf8.DecodeLastRuneInString(prefix[:start])
		if !unicode.IsLetter(r) {
			break
		}
		start -= size
	}
	return prefix[start:]
}
