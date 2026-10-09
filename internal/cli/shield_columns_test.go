// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

package cli

import (
	"context"
	"path/filepath"
	"strings"
	"testing"

	"github.com/mixlayer/cli/internal/store"
)

const columnsCSV = `customer,email,company,plan,note
Jordan Avery,jordan@northwind.example,Northwind Traders,Team,"Asked twice how to export data"
Marcus Oyelaran,marcus@fieldstone.example,Fieldstone Labs,Team,"Power user"
Elena Sokolova,elena@brightpath.example,Brightpath Logistics,Business,"Champion left in August"
`

func TestCSVColumnTerms(t *testing.T) {
	terms, err := csvColumnTerms(columnsCSV, []string{"Company", "customer"})
	if err != nil {
		t.Fatal(err)
	}
	got := map[string]string{}
	for _, term := range terms {
		got[term.Term] = term.Kind
	}
	if got["Fieldstone Labs"] != "COMPANY" || got["Jordan Avery"] != "CUSTOMER" || len(terms) != 6 {
		t.Fatalf("terms = %#v", terms)
	}
	if _, err := csvColumnTerms(columnsCSV, []string{"ssn"}); err == nil || !strings.Contains(err.Error(), "the CSV has: customer, email") {
		t.Fatalf("missing column error = %v", err)
	}
	if _, err := csvColumnTerms("", []string{"company"}); err == nil {
		t.Fatal("expected an error without data")
	}
	if columnKind("account name") != "ACCOUNT_NAME" || columnKind("2nd-email") != "ND_EMAIL" || columnKind("123") != "FIELD" {
		t.Fatalf("columnKind: %q %q %q", columnKind("account name"), columnKind("2nd-email"), columnKind("123"))
	}
}

func TestShieldAskMaskColumnsKeepsCompaniesOut(t *testing.T) {
	s, err := store.OpenWithContext(context.Background(), filepath.Join(t.TempDir(), "m.db"))
	if err != nil {
		t.Fatal(err)
	}
	defer s.Close()
	terms, err := csvColumnTerms(columnsCSV, []string{"customer", "company"})
	if err != nil {
		t.Fatal(err)
	}
	prepared, err := prepareShieldAskPayloadWith(context.Background(), s, columnsCSV, "Which accounts at Fieldstone Labs or elsewhere might churn?", detectorRuntime{Name: "regex", ExtraTerms: terms})
	if err != nil {
		t.Fatal(err)
	}
	for _, private := range []string{"Northwind", "Fieldstone", "Brightpath", "Jordan Avery", "Oyelaran", "jordan@northwind.example"} {
		if strings.Contains(prepared.Payload, private) {
			t.Errorf("payload still contains %q:\n%s", private, prepared.Payload)
		}
	}
	for _, kept := range []string{"Champion left in August", "Business", "COMPANY_", "CUSTOMER_"} {
		if !strings.Contains(prepared.Payload, kept) {
			t.Errorf("payload lost %q:\n%s", kept, prepared.Payload)
		}
	}
	if len(prepared.Leaks) != 0 {
		t.Errorf("tripwire hits: %#v", prepared.Leaks)
	}
}
