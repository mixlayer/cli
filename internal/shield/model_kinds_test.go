// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

package shield

import (
	"context"
	"testing"
)

func TestModelDetectorKeepsUnknownKinds(t *testing.T) {
	text := "Reach Jordan Avery at jordan@acme.example."
	call := func(context.Context, string) (string, error) {
		return `{"entities":[{"kind":"EMAIL_ADDRESS","text":"jordan@acme.example"},{"kind":"PERSON","text":"Jordan Avery"},{"kind":"PERSON","text":""}]}`, nil
	}
	ents, _, ignored, err := DetectWithStats(context.Background(), text, nil, call, false)
	if err != nil {
		t.Fatalf("detector failed: %v", err)
	}
	var email bool
	for _, e := range ents {
		if e.Value == "jordan@acme.example" {
			email = true
		}
	}
	if !email || ignored < 1 {
		t.Fatalf("entities=%+v ignored=%d", ents, ignored)
	}
}
