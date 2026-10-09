// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

// Package systemone implements Jini's typed decision wire contract.
package systemone

import (
	"bytes"
	"encoding/json"
	"fmt"
	"io"
	"math"
	"sort"
	"strconv"
	"strings"
)

const (
	MaxBody      = 8 * 1024 * 1024
	MaxState     = 4 * 1024 * 1024
	MaxQuestion  = 16 * 1024
	MaxQuestions = 4096
)

type Question struct {
	Type         string          `json:"type"`
	Instructions json.RawMessage `json:"instructions"`
	Criteria     json.RawMessage `json:"criteria,omitempty"`
}
type Questions map[string]Question

type Request struct {
	Model     string          `json:"model"`
	State     json.RawMessage `json:"state"`
	Questions Questions       `json:"questions"`
}
type Answer struct {
	Type          string             `json:"type"`
	Noul          *float64           `json:"noul,omitempty"`
	Choice        *string            `json:"choice,omitempty"`
	Score         *float64           `json:"score,omitempty"`
	Legend        map[string]string  `json:"legend,omitempty"`
	Probabilities map[string]float64 `json:"probabilities,omitempty"`
	Confidence    *float64           `json:"confidence,omitempty"`
}
type Usage struct {
	InputTokens  uint64 `json:"input_tokens"`
	OutputTokens uint64 `json:"output_tokens"`
}
type Response struct {
	Model   string            `json:"model"`
	Answers map[string]Answer `json:"answers"`
	Usage   *Usage            `json:"usage"`
}

func JSON(v any) json.RawMessage { b, _ := json.Marshal(v); return b }

// State detects structured file contents only. Scalar JSON remains literal text.
func State(data []byte) json.RawMessage {
	b := bytes.TrimSpace(data)
	if len(b) > 0 && (b[0] == '{' || b[0] == '[') && json.Valid(b) {
		return b
	}
	return JSON(string(data))
}

// ParseQuestions rejects duplicate IDs instead of silently keeping the last one.
func ParseQuestions(data []byte) (Questions, error) {
	d := json.NewDecoder(bytes.NewReader(data))
	tok, err := d.Token()
	if err != nil || tok != json.Delim('{') {
		return nil, fmt.Errorf("questions must be a JSON object")
	}
	out := Questions{}
	for d.More() {
		tok, err = d.Token()
		if err != nil {
			return nil, err
		}
		id := tok.(string)
		if _, exists := out[id]; exists {
			return nil, fmt.Errorf("duplicate question ID %q", id)
		}
		var q Question
		d.DisallowUnknownFields()
		if err = d.Decode(&q); err != nil {
			return nil, fmt.Errorf("question %q: %w", id, err)
		}
		out[id] = q
	}
	if _, err = d.Token(); err != nil {
		return nil, err
	}
	var extra any
	if err = d.Decode(&extra); err != io.EOF {
		return nil, fmt.Errorf("questions must contain one JSON object")
	}
	return out, nil
}

// render matches Jini render_entry: strings verbatim, other JSON compact and sorted.
func render(raw json.RawMessage) (string, error) {
	var v any
	d := json.NewDecoder(bytes.NewReader(raw))
	d.UseNumber()
	if err := d.Decode(&v); err != nil || !json.Valid(raw) {
		return "", fmt.Errorf("invalid JSON description")
	}
	switch v.(type) {
	case string, map[string]any, []any, nil:
	default:
		return "", fmt.Errorf("description must be text, object, array or null")
	}
	if s, ok := v.(string); ok {
		return s, nil
	}
	var b bytes.Buffer
	e := json.NewEncoder(&b)
	e.SetEscapeHTML(false)
	if err := e.Encode(v); err != nil {
		return "", err
	}
	return strings.TrimSuffix(b.String(), "\n"), nil
}

func (r Request) Validate() error {
	if strings.TrimSpace(r.Model) == "" || len(r.Model) > 256 {
		return fmt.Errorf("model must be nonempty and at most 256 bytes")
	}
	state := bytes.TrimSpace(r.State)
	if len(state) == 0 || (state[0] != '"' && state[0] != '{' && state[0] != '[') {
		return fmt.Errorf("state must be text, object or array")
	}
	text, err := render(state)
	if err != nil || strings.TrimSpace(text) == "" || len(text) > MaxState {
		return fmt.Errorf("rendered state must be nonempty and at most 4 MiB")
	}
	if len(r.Questions) < 1 || len(r.Questions) > MaxQuestions {
		return fmt.Errorf("questions must contain 1–4096 entries")
	}
	branches := 0
	for id, q := range r.Questions {
		if strings.TrimSpace(id) == "" || len(id) > 256 {
			return fmt.Errorf("question ID must be nonblank and at most 256 bytes")
		}
		n, err := q.validate()
		if err != nil {
			return fmt.Errorf("question %q: %w", id, err)
		}
		branches += n
	}
	if branches > MaxQuestions {
		return fmt.Errorf("expanded question count exceeds 4096")
	}
	body, err := json.Marshal(r)
	if err != nil {
		return err
	}
	if len(body) > MaxBody {
		return fmt.Errorf("request body exceeds 8 MiB")
	}
	return nil
}

func (q Question) validate() (int, error) {
	instructions, err := render(q.Instructions)
	if err != nil {
		return 0, err
	}
	check := func(s string) error {
		if strings.TrimSpace(s) == "" || len(s) > MaxQuestion {
			return fmt.Errorf("expanded question must be nonempty and at most 16 KiB")
		}
		return nil
	}
	switch q.Type {
	case "noul":
		if len(q.Criteria) > 0 && string(bytes.TrimSpace(q.Criteria)) != "null" {
			var criteria map[string]json.RawMessage
			if err := json.Unmarshal(q.Criteria, &criteria); err != nil {
				return 0, fmt.Errorf("noul criteria must be an object")
			}
			// Null optional rubric entries are omitted by Jini's NoulCriteria.
			for k, v := range criteria {
				if k != "true" && k != "false" {
					return 0, fmt.Errorf("noul criteria accepts only true and false")
				}
				if _, err := render(v); err != nil {
					return 0, err
				}
				if string(bytes.TrimSpace(v)) == "null" {
					delete(criteria, k)
				}
			}
			rubric, _ := render(JSON(criteria))
			instructions += "\n\nYes/no rubric:\n" + rubric
		}
		return 1, check(instructions)
	case "choice", "score":
		options := map[string]json.RawMessage{}
		var labels []string
		if q.Type == "choice" {
			if err := json.Unmarshal(q.Criteria, &options); err != nil || len(options) < 1 || len(options) > 255 {
				return 0, fmt.Errorf("choice requires 1–255 described options")
			}
			for k := range options {
				if strings.TrimSpace(k) == "" {
					return 0, fmt.Errorf("choice labels must be nonblank")
				}
				labels = append(labels, k)
			}
			sort.Strings(labels)
		} else {
			var levels []json.RawMessage
			if err := json.Unmarshal(q.Criteria, &levels); err != nil || len(levels) < 2 || len(levels) > 10 {
				return 0, fmt.Errorf("score requires 2–10 described levels")
			}
			for i, v := range levels {
				k := strconv.Itoa(i)
				labels = append(labels, k)
				options[k] = v
			}
		}
		rubric := make([]string, 0, len(labels))
		for _, k := range labels {
			v, err := render(options[k])
			if err != nil {
				return 0, err
			}
			rubric = append(rubric, "- "+k+": "+v)
		}
		common := instructions + "\n\nAnswer rubric (choose exactly one option):\n" + strings.Join(rubric, "\n")
		for _, k := range labels {
			if err := check(common + "\n\nDoes the following option correctly answer the question under the full rubric?\n" + k); err != nil {
				return 0, err
			}
		}
		return len(labels), nil
	default:
		return 0, fmt.Errorf("type must be noul, choice or score")
	}
}

func unit(v float64) bool { return !math.IsNaN(v) && !math.IsInf(v, 0) && v >= 0 && v <= 1 }

// ValidateResponse fails closed for missing, mistyped, or inconsistent answers.
func (r Request) ValidateResponse(res Response) error {
	if res.Model != r.Model || len(res.Answers) != len(r.Questions) || res.Usage == nil || res.Usage.OutputTokens != 0 {
		return fmt.Errorf("invalid systemone response envelope")
	}
	for id, q := range r.Questions {
		a, ok := res.Answers[id]
		valid := ok && a.Type == q.Type
		switch q.Type {
		case "noul":
			valid = valid && a.Noul != nil && unit(*a.Noul)
		case "choice", "score":
			valid = valid && a.Confidence != nil && unit(*a.Confidence)
			sum := 0.0
			for _, p := range a.Probabilities {
				valid = valid && unit(p)
				sum += p
			}
			valid = valid && math.Abs(sum-1) < 1e-8
			if q.Type == "choice" {
				var criteria map[string]json.RawMessage
				_ = json.Unmarshal(q.Criteria, &criteria)
				valid = valid && len(criteria) == len(a.Probabilities) && a.Choice != nil
				for k := range criteria {
					_, ok := a.Probabilities[k]
					valid = valid && ok
				}
				if a.Choice != nil {
					p, ok := a.Probabilities[*a.Choice]
					valid = valid && ok
					for _, v := range a.Probabilities {
						valid = valid && v <= p
					}
				}
			} else {
				var levels []json.RawMessage
				_ = json.Unmarshal(q.Criteria, &levels)
				valid = valid && len(levels) == len(a.Probabilities) && len(levels) == len(a.Legend) && a.Score != nil
				expected := 0.0
				for i, v := range levels {
					k := strconv.Itoa(i)
					p, ok := a.Probabilities[k]
					legend, _ := render(v)
					actual, present := a.Legend[k]
					valid = valid && ok && present && actual == legend
					expected += float64(i) * p
				}
				valid = valid && a.Score != nil && math.Abs(*a.Score-expected) < 1e-8
			}
		}
		if !valid {
			return fmt.Errorf("invalid systemone answer for %q", id)
		}
	}
	return nil
}
