// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.
package systemone

import (
	"fmt"
	"strings"
	"testing"
)

func TestRequestLimits(t *testing.T) {
	valid := func() Request {
		return Request{Model: "fixture", State: JSON("facts"), Questions: Questions{"q": {Type: "noul", Instructions: JSON("?")}}}
	}
	for _, tc := range []struct {
		name string
		edit func(*Request)
	}{
		{"empty questions", func(r *Request) { r.Questions = Questions{} }},
		{"too many questions", func(r *Request) {
			for i := 0; i < MaxQuestions; i++ {
				r.Questions[fmt.Sprint(i)] = Question{Type: "noul", Instructions: JSON("?")}
			}
		}},
		{"expanded count", func(r *Request) {
			r.Questions = Questions{}
			for i := 0; i < 2049; i++ {
				r.Questions[fmt.Sprint(i)] = Question{Type: "score", Instructions: JSON("?"), Criteria: JSON([]string{"a", "b"})}
			}
		}},
		{"body bytes", func(r *Request) { r.State = JSON(strings.Repeat("\x01", 2*1024*1024)) }},
		{"state bytes", func(r *Request) { r.State = JSON(strings.Repeat("x", MaxState+1)) }},
		{"expanded question bytes", func(r *Request) {
			r.Questions["q"] = Question{Type: "score", Instructions: JSON(strings.Repeat("x", MaxQuestion-1)), Criteria: JSON([]string{"a", "b"})}
		}},
		{"empty id", func(r *Request) { r.Questions[" "] = r.Questions["q"] }},
		{"long id", func(r *Request) { r.Questions[strings.Repeat("a", 257)] = r.Questions["q"] }},
		{"scalar state", func(r *Request) { r.State = JSON(42) }},
		{"empty state", func(r *Request) { r.State = JSON(" ") }},
		{"scalar instructions", func(r *Request) { r.Questions["q"] = Question{Type: "noul", Instructions: JSON(true)} }},
		{"empty instructions", func(r *Request) { r.Questions["q"] = Question{Type: "noul", Instructions: JSON("")} }},
		{"bad rubric", func(r *Request) {
			r.Questions["q"] = Question{Type: "noul", Instructions: JSON("?"), Criteria: JSON(map[string]string{"maybe": "yes"})}
		}},
	} {
		t.Run(tc.name, func(t *testing.T) {
			r := valid()
			tc.edit(&r)
			if err := r.Validate(); err == nil {
				t.Fatal("expected validation error")
			}
		})
	}
	r := valid()
	if err := r.Validate(); err != nil {
		t.Fatal(err)
	}
	r.State = JSON(strings.Repeat("x", MaxState))
	r.Questions["q"] = Question{Type: "noul", Instructions: JSON(strings.Repeat("x", MaxQuestion))}
	if err := r.Validate(); err != nil {
		t.Fatalf("inclusive bounds: %v", err)
	}
}

func TestQuestionsRejectUnknownFields(t *testing.T) {
	for _, s := range []string{`{"q":{"type":"noul","instructions":"?","unexpected":1}}`, `null`, `{}`, `{"q":{"type":"noul","instructions":"?"}} {}`} {
		q, err := ParseQuestions([]byte(s))
		if err == nil {
			err = (Request{Model: "fixture", State: JSON("facts"), Questions: q}).Validate()
		}
		if err == nil {
			t.Fatalf("accepted %s", s)
		}
	}
}
