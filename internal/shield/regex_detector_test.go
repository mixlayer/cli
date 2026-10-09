package shield

import "testing"

func TestExpandedRegexFloor(t *testing.T) {
	cases := []struct{ text, kind string }{
		{"GB82 WEST 1234 5698 7654 32", "ACCOUNT"},
		{"ZZ99 MIXL 0000 0000 0000 00", "ACCOUNT"},
		{"+44 20 7946 0991", "PHONE"},
		{"+61 (2) 5550-0199", "PHONE"},
		{"020 7946 0991", "PHONE"},
		{"jane dot roe at example dot com", "EMAIL"},
		{"jane[at]example.com", "EMAIL"},
		{"jane(at)example.com", "EMAIL"},
		{"XX 00 11 22 C", "ID_NUMBER"},
		{"000 111 222", "ID_NUMBER"},
	}
	for _, tc := range cases {
		t.Run(tc.text, func(t *testing.T) {
			for _, entity := range Detect(tc.text) {
				if entity.Kind == tc.kind && entity.Value == tc.text {
					return
				}
			}
			t.Fatalf("%q not fully detected as %s: %#v", tc.text, tc.kind, Detect(tc.text))
		})
	}
}

func TestIBANChecksumChangesRiskButDoesNotDropCandidate(t *testing.T) {
	valid, invalid := Detect("GB82 WEST 1234 5698 7654 32"), Detect("GB83 WEST 1234 5698 7654 32")
	if len(valid) != 1 || len(invalid) != 1 || valid[0].Kind != "ACCOUNT" || invalid[0].Kind != "ACCOUNT" || valid[0].Risk <= invalid[0].Risk {
		t.Fatalf("valid=%#v invalid=%#v", valid, invalid)
	}
}
