package shield

import (
	"context"
	"path/filepath"
	"strings"
	"testing"

	"github.com/mixlayer/cli/internal/store"
)

func TestModelSpansRegexFloorAndFallback(t *testing.T) {
	text := "Zora Quill sent zora@fiction.example to Zora Quill"
	call := func(context.Context, string) (string, error) {
		return `{"entities":[{"kind":"PERSON","text":"Zora Quill"},{"kind":"ADDRESS","text":"not in input"}]}`, nil
	}
	got, fallback, err := DetectWith(context.Background(), text, nil, call, false)
	if err != nil || fallback {
		t.Fatalf("detect: %v, fallback %v", err, fallback)
	}
	var people, email, phantom int
	for _, e := range got {
		if e.Kind == "PERSON" && e.Value == "Zora Quill" {
			people++
			if text[e.Start:e.End] != e.Value {
				t.Fatal("bad offset")
			}
		}
		if e.Kind == "EMAIL" {
			email++
		}
		if e.Kind == "ADDRESS" {
			phantom++
		}
	}
	if people != 2 || email != 1 || phantom != 0 {
		t.Fatalf("entities = %#v", got)
	}
	partial := func(context.Context, string) (string, error) {
		return `{"entities":[{"kind":"PERSON","text":"Zora"}]}`, nil
	}
	covered, _, err := DetectWith(context.Background(), "Zora Quill", nil, partial, false)
	if err != nil || len(covered) != 1 || covered[0].Value != "Zora Quill" {
		t.Fatalf("regex floor = %#v, %v", covered, err)
	}
	bad := func(context.Context, string) (string, error) { return "not json", nil }
	if _, _, err := DetectWith(context.Background(), text, nil, bad, false); err == nil {
		t.Fatal("malformed response did not fail closed")
	}
	withFallback, used, err := DetectWith(context.Background(), text, nil, bad, true)
	if err != nil || !used || len(withFallback) == 0 {
		t.Fatalf("fallback = %#v, %v, %v", withFallback, used, err)
	}
}

func TestModelDetectorCountsIgnoredNonSubstringSpans(t *testing.T) {
	_, _, ignored, err := DetectWithStats(context.Background(), "Zora Quill", nil, func(context.Context, string) (string, error) {
		return `{"entities":[{"kind":"PERSON","text":"not present"}]}`, nil
	}, false)
	if err != nil || ignored != 1 {
		t.Fatalf("ignored=%d err=%v", ignored, err)
	}
}

func TestModelDetectorChunksOnBoundaryAndDedupes(t *testing.T) {
	text := strings.Repeat("é", 5900) + "\nZora Quill\n" + strings.Repeat("é", 6100)
	parts := chunks(text)
	if len(parts) < 2 {
		t.Fatalf("chunks = %d", len(parts))
	}
	for _, part := range parts {
		if len([]rune(part)) > 6000 {
			t.Fatalf("chunk has %d runes", len([]rune(part)))
		}
	}
	calls := 0
	model := func(context.Context, string) (string, error) {
		calls++
		return `{"entities":[{"kind":"PERSON","text":"Zora Quill"}]}`, nil
	}
	got, _, err := DetectWith(context.Background(), text, nil, model, false)
	if err != nil {
		t.Fatal(err)
	}
	people := 0
	for _, e := range got {
		if e.Value == "Zora Quill" {
			people++
		}
	}
	if calls != len(parts) || people != 1 {
		t.Fatalf("calls=%d people=%d entities=%#v", calls, people, got)
	}
}

func TestDictionaryMasksCaseInsensitiveWordBoundary(t *testing.T) {
	ctx := context.Background()
	s, err := store.Open(filepath.Join(t.TempDir(), "vault.db"))
	if err != nil {
		t.Fatal(err)
	}
	defer s.Close()
	terms := []store.DictionaryTerm{{Term: "moon pebble", Kind: "PERSON"}}
	text := "Moon Pebble and moon pebblefish"
	entities, _, err := DetectWith(ctx, text, terms, nil, false)
	if err != nil {
		t.Fatal(err)
	}
	res, err := RedactEntities(ctx, s, text, entities, false)
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(res.Text, "Moon Pebble") || !strings.Contains(res.Text, "moon pebblefish") {
		t.Fatalf("masked = %q", res.Text)
	}
	whole, _, err := DetectWith(ctx, "Jane Roe", []store.DictionaryTerm{{Term: "Jane", Kind: "PERSON"}}, nil, false)
	if err != nil || len(whole) != 1 || whole[0].Value != "Jane Roe" {
		t.Fatalf("dictionary overlap = %#v, %v", whole, err)
	}
}

func TestRehydrateCheckedRepairsAndReportsUnknown(t *testing.T) {
	ctx := context.Background()
	s, err := store.Open(filepath.Join(t.TempDir(), "vault.db"))
	if err != nil {
		t.Fatal(err)
	}
	defer s.Close()
	if err := s.SaveVaultEntry(ctx, store.VaultEntry{Token: "EMAIL_abcdef12", Value: "zora@fiction.example", Kind: "EMAIL"}); err != nil {
		t.Fatal(err)
	}
	got, err := RehydrateChecked(ctx, s, "EMAIL_abcdef12 email_ABCDEF12 EMAIL_ffff0000")
	if err != nil {
		t.Fatal(err)
	}
	if got.RestoredCount != 2 || len(got.Repairs) != 1 || len(got.UnrestoredTokens) != 1 || got.UnrestoredTokens[0] != "EMAIL_ffff0000" {
		t.Fatalf("report = %#v", got)
	}
	spaced, err := RehydrateCheckedForTokens(ctx, s, "EMAIL_ ab cd ef 12", []string{"EMAIL_abcdef12"})
	if err != nil || spaced.RestoredCount != 1 || len(spaced.Repairs) != 1 {
		t.Fatalf("spaced = %#v, %v", spaced, err)
	}
	if err := s.SaveVaultEntry(ctx, store.VaultEntry{Token: "email_ABCDEF12", Value: "other@fiction.example", Kind: "EMAIL"}); err != nil {
		t.Fatal(err)
	}
	ambiguous, err := RehydrateChecked(ctx, s, "Email_AbCdEf12")
	if err != nil {
		t.Fatal(err)
	}
	if ambiguous.RestoredCount != 0 || len(ambiguous.UnrestoredTokens) != 1 {
		t.Fatalf("ambiguous = %#v", ambiguous)
	}
	scoped, err := RehydrateCheckedForTokens(ctx, s, "EMAIL_abcdef12 email_ABCDEF12", []string{"EMAIL_abcdef12"})
	if err != nil || scoped.RestoredCount != 1 || scoped.UnrestoredCount != 1 {
		t.Fatalf("scoped = %#v, %v", scoped, err)
	}
}

func TestRehydrateRepairsRelabelledPlaceholder(t *testing.T) {
	ctx := context.Background()
	s, err := store.OpenWithContext(ctx, filepath.Join(t.TempDir(), "r.db"))
	if err != nil {
		t.Fatal(err)
	}
	defer s.Close()
	for _, e := range []store.VaultEntry{
		{Token: "COMPANY_568586ad", Value: "Kestrel Analytics", Kind: "COMPANY"},
		{Token: "CUSTOMER_1234abcd", Value: "Hiroshi Tanaka", Kind: "CUSTOMER"},
		{Token: "EMAIL_99990000", Value: "x@other.example", Kind: "EMAIL"},
	} {
		if err := s.SaveVaultEntry(ctx, e); err != nil {
			t.Fatal(err)
		}
	}
	allowed := []string{"COMPANY_568586ad", "CUSTOMER_1234abcd"}
	got, err := RehydrateCheckedForTokens(ctx, s, "CUSTOMER_568586ad (CUSTOMER_1234abcd) and PERSON_99990000", allowed)
	if err != nil {
		t.Fatal(err)
	}
	if got.Text != "Kestrel Analytics (Hiroshi Tanaka) and PERSON_99990000" || got.RestoredCount != 2 || got.UnrestoredCount != 1 {
		t.Fatalf("got %#v", got)
	}
}
