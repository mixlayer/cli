// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

package shield

import "testing"

func TestLikelyNamesMaskWholeName(t *testing.T) {
	cases := []struct {
		text, want string
	}{
		{"Contact Jordan Avery at the office.", "Jordan Avery"},
		{"Send this to Jordan Avery tomorrow.", "Jordan Avery"},
		{"Hi Priya, thanks for the update.", "Priya"},
		{"Please call Zorra Quill tomorrow.", "Zorra Quill"},
		{"Mock account belongs to Pebble.", "Pebble"},
		{"Contact Zora\nQuill about the case.", "Zora\nQuill"},
		{"Ask Art about the form.", "Art"},
	}
	for _, tc := range cases {
		got := detectLikelyNames(tc.text)
		if len(got) != 1 || got[0].Value != tc.want {
			t.Errorf("%q: got %+v, want one PERSON %q", tc.text, got, tc.want)
		}
	}
}

func TestLikelyNamesSkipsCommonPhrases(t *testing.T) {
	for _, text := range []string{
		"The quarterly report covers Product Analytics and the Growth Team roadmap.",
		"Please review the Pull Request before Friday.",
		"Our offices are in San Francisco and near Union Square.",
		"Today we shipped it.",
		"Zora lives at 123 Imaginary Lane.",
	} {
		if got := detectLikelyNames(text); len(got) != 0 {
			t.Errorf("%q: unexpected PERSON %+v", text, got)
		}
	}
}
