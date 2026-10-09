use anyhow::{Result, anyhow};

use super::*;

fn has(entities: &[Entity], kind: &str, value: &str) -> bool {
    entities.iter().any(|e| e.kind == kind && e.value == value)
}

#[test]
fn detect_finds_deterministic_pii() {
    let e = detect("Jordan Avery used jordan@example.com, 4111 1111 1111 1111, 192.168.1.1, and 123-45-6789.");
    for kind in ["PERSON", "EMAIL", "CARD", "IP", "SSN"] {
        assert!(e.iter().any(|x| x.kind == kind), "{kind}: {e:?}");
    }
    assert_eq!(risk_score(&e), 10);
}
#[test]
fn detect_likely_names_skips_common_non_person_phrases() {
    let e = detect("Trips to New York and San Francisco used Google Cloud logs. Alice Johnson still owns the account.");
    for value in ["New York", "San Francisco", "Google Cloud"] {
        assert!(!has(&e, "PERSON", value));
    }
    assert!(has(&e, "PERSON", "Alice Johnson"));
}
#[test]
fn detect_prefers_higher_risk_overlap() {
    let e = detect("Alice Bob@example.com");
    assert_eq!(e.len(), 1);
    assert!(has(&e, "EMAIL", "Bob@example.com"));
}
#[test]
fn restructure_coarsens_without_bucketing_dates() {
    let opts = RestructureOptions {
        bucket_numerics: true,
        coarsen_dates: "quarter".into(),
        drop_columns: vec!["email".into()],
    };
    let got = restructure(
        "name,email,amount,date\nJordan Avery,jordan@example.com,1234,2026-06-24\n",
        &opts,
    )
    .unwrap();
    assert!(!got.contains("example.com"));
    assert!(got.contains("1000-9999"));
    assert!(got.contains("2026-Q2"));
}
#[test]
fn model_detector_keeps_unknown_kinds() {
    let text = "Reach Jordan Avery at jordan@acme.example.";
    let call = |_: &str| {
        Ok(r#"{"entities":[{"kind":"EMAIL_ADDRESS","text":"jordan@acme.example"},{"kind":"PERSON","text":"Jordan Avery"},{"kind":"PERSON","text":""}]}"#.into())
    };
    let (e, _, ignored) = detect_with_stats(text, &[], Some(&call), false).unwrap();
    assert!(e.iter().any(|x| x.value == "jordan@acme.example"));
    assert!(ignored >= 1);
}
#[test]
fn likely_names_mask_whole_name() {
    for (text, want) in [
        ("Contact Jordan Avery at the office.", "Jordan Avery"),
        ("Send this to Jordan Avery tomorrow.", "Jordan Avery"),
        ("Hi Priya, thanks for the update.", "Priya"),
        ("Please call Zorra Quill tomorrow.", "Zorra Quill"),
        ("Mock account belongs to Pebble.", "Pebble"),
        ("Contact Zora\nQuill about the case.", "Zora\nQuill"),
        ("Ask Art about the form.", "Art"),
    ] {
        let got = detect_likely_names(text);
        assert_eq!(got.len(), 1, "{text}: {got:?}");
        assert_eq!(got[0].value, want);
    }
}
#[test]
fn likely_names_skips_common_phrases() {
    for text in [
        "The quarterly report covers Product Analytics and the Growth Team roadmap.",
        "Please review the Pull Request before Friday.",
        "Our offices are in San Francisco and near Union Square.",
        "Today we shipped it.",
        "Zora lives at 123 Imaginary Lane.",
    ] {
        assert!(detect_likely_names(text).is_empty(), "{text}");
    }
}
#[test]
fn expanded_regex_floor() {
    for (text, kind) in [
        ("GB82 WEST 1234 5698 7654 32", "ACCOUNT"),
        ("ZZ99 MIXL 0000 0000 0000 00", "ACCOUNT"),
        ("+44 20 7946 0991", "PHONE"),
        ("+61 (2) 5550-0199", "PHONE"),
        ("020 7946 0991", "PHONE"),
        ("jane dot roe at example dot com", "EMAIL"),
        ("jane[at]example.com", "EMAIL"),
        ("jane(at)example.com", "EMAIL"),
        ("XX 00 11 22 C", "ID_NUMBER"),
        ("000 111 222", "ID_NUMBER"),
    ] {
        assert!(has(&detect(text), kind, text), "{text}: {:?}", detect(text));
    }
}
#[test]
fn iban_checksum_changes_risk_but_does_not_drop_candidate() {
    let valid = detect("GB82 WEST 1234 5698 7654 32");
    let invalid = detect("GB83 WEST 1234 5698 7654 32");
    assert_eq!(valid.len(), 1);
    assert_eq!(invalid.len(), 1);
    assert_eq!(valid[0].kind, "ACCOUNT");
    assert_eq!(invalid[0].kind, "ACCOUNT");
    assert!(valid[0].risk > invalid[0].risk);
}
const COLUMNS_CSV: &str = "customer,email,company,plan,note\nJordan Avery,jordan@northwind.example,Northwind Traders,Team,\"Asked twice how to export data\"\nMarcus Oyelaran,marcus@fieldstone.example,Fieldstone Labs,Team,\"Power user\"\nElena Sokolova,elena@brightpath.example,Brightpath Logistics,Business,\"Champion left in August\"\n";
#[test]
fn csv_column_terms_case() {
    let terms = csv_column_terms(COLUMNS_CSV, &["Company", "customer"]).unwrap();
    assert_eq!(terms.len(), 6);
    assert!(terms.iter().any(|t| t.term == "Fieldstone Labs" && t.kind == "COMPANY"));
    assert!(terms.iter().any(|t| t.term == "Jordan Avery" && t.kind == "CUSTOMER"));
    assert!(
        csv_column_terms(COLUMNS_CSV, &["ssn"])
            .unwrap_err()
            .to_string()
            .contains("the CSV has: customer, email")
    );
    assert!(csv_column_terms("", &["company"]).is_err());
    assert_eq!(column_kind("account name"), "ACCOUNT_NAME");
    assert_eq!(column_kind("2nd-email"), "ND_EMAIL");
    assert_eq!(column_kind("123"), "FIELD");
}
#[test]
fn csv_column_terms_accepts_go_lazy_quotes_and_leading_space() {
    let data = "name,company\nAlice,   \"North, Wind\"\nBob,North\"wind\nCara,\"Field\"stone\"\n";
    let terms = csv_column_terms(data, &["company"]).unwrap();
    assert_eq!(
        terms.iter().map(|t| t.term.as_str()).collect::<Vec<_>>(),
        ["North, Wind", "North\"wind", "Field\"stone"]
    );
}
#[test]
fn shield_ask_mask_columns_keeps_companies_out() {
    let terms = csv_column_terms(COLUMNS_CSV, &["customer", "company"]).unwrap();
    let prompt = "Which accounts at Fieldstone Labs or elsewhere might churn?";
    let payload = format!("{COLUMNS_CSV}\n{prompt}");
    let (entities, _) = detect_with(&payload, &terms, None, false).unwrap();
    let prepared = redact_entities(&mut MemoryVault::default(), &payload, entities, false).unwrap();
    for private in [
        "Northwind",
        "Fieldstone",
        "Brightpath",
        "Jordan Avery",
        "Oyelaran",
        "jordan@northwind.example",
    ] {
        assert!(!prepared.text.contains(private), "{private}: {}", prepared.text);
    }
    for kept in ["Champion left in August", "Business", "COMPANY_", "CUSTOMER_"] {
        assert!(prepared.text.contains(kept), "{kept}: {}", prepared.text);
    }
}
#[test]
fn model_spans_regex_floor_and_fallback() {
    let text = "Zora Quill sent zora@fiction.example to Zora Quill";
    let call = |_: &str| {
        Ok(r#"{"entities":[{"kind":"PERSON","text":"Zora Quill"},{"kind":"ADDRESS","text":"not in input"}]}"#.into())
    };
    let (e, fallback) = detect_with(text, &[], Some(&call), false).unwrap();
    assert!(!fallback);
    assert_eq!(
        e.iter()
            .filter(|e| has(std::slice::from_ref(e), "PERSON", "Zora Quill"))
            .count(),
        2
    );
    assert_eq!(e.iter().filter(|e| e.kind == "EMAIL").count(), 1);
    assert!(!e.iter().any(|e| e.kind == "ADDRESS"));
    for entity in &e {
        assert_eq!(&text[entity.start..entity.end], entity.value);
    }
    let partial = |_: &str| Ok(r#"{"entities":[{"kind":"PERSON","text":"Zora"}]}"#.into());
    let (covered, _) = detect_with("Zora Quill", &[], Some(&partial), false).unwrap();
    assert_eq!(covered.len(), 1);
    assert_eq!(covered[0].value, "Zora Quill");
    let bad = |_: &str| Ok("not json".into());
    assert!(detect_with(text, &[], Some(&bad), false).is_err());
    let (e, used) = detect_with(text, &[], Some(&bad), true).unwrap();
    assert!(used && !e.is_empty());
}
#[test]
fn model_detector_counts_ignored_non_substring_spans() {
    let call = |_: &str| Ok(r#"{"entities":[{"kind":"PERSON","text":"not present"}]}"#.into());
    let (_, _, ignored) = detect_with_stats("Zora Quill", &[], Some(&call), false).unwrap();
    assert_eq!(ignored, 1);
}
#[test]
fn model_detector_chunks_on_boundary_and_dedupes() {
    let text = format!("{}\nZora Quill\n{}", "é".repeat(5900), "é".repeat(6100));
    let parts = chunks(&text);
    assert!(parts.len() >= 2);
    assert!(parts.iter().all(|p| p.chars().count() <= 6000));
    let calls = std::cell::Cell::new(0);
    let call = |_: &str| {
        calls.set(calls.get() + 1);
        Ok(r#"{"entities":[{"kind":"PERSON","text":"Zora Quill"}]}"#.into())
    };
    let (got, _) = detect_with(&text, &[], Some(&call), false).unwrap();
    assert_eq!(calls.get(), parts.len());
    assert_eq!(got.iter().filter(|e| e.value == "Zora Quill").count(), 1);
}
#[test]
fn dictionary_masks_case_insensitive_word_boundary() {
    let terms = [DictionaryTerm {
        term: "moon pebble".into(),
        kind: "PERSON".into(),
    }];
    let text = "Moon Pebble and moon pebblefish";
    let (e, _) = detect_with(text, &terms, None, false).unwrap();
    let res = redact_entities(&mut MemoryVault::default(), text, e, false).unwrap();
    assert!(!res.text.contains("Moon Pebble"));
    assert!(res.text.contains("moon pebblefish"));
    let terms = [DictionaryTerm {
        term: "Jane".into(),
        kind: "PERSON".into(),
    }];
    let (e, _) = detect_with("Jane Roe", &terms, None, false).unwrap();
    assert_eq!(e.len(), 1);
    assert_eq!(e[0].value, "Jane Roe");
}
#[test]
fn rehydrate_checked_repairs_and_reports_unknown() {
    let mut v = MemoryVault::default();
    v.save_entry(VaultEntry {
        token: "EMAIL_abcdef12".into(),
        value: "zora@fiction.example".into(),
        kind: "EMAIL".into(),
    })
    .unwrap();
    let got = rehydrate_checked(&v, "EMAIL_abcdef12 email_ABCDEF12 EMAIL_ffff0000").unwrap();
    assert_eq!(got.restored_count, 2);
    assert_eq!(got.repairs.len(), 1);
    assert_eq!(got.unrestored_tokens, ["EMAIL_ffff0000"]);
    let spaced = rehydrate_checked_for_tokens(&v, "EMAIL_ ab cd ef 12", &["EMAIL_abcdef12"]).unwrap();
    assert_eq!(spaced.restored_count, 1);
    assert_eq!(spaced.repairs.len(), 1);
    v.save_entry(VaultEntry {
        token: "email_ABCDEF12".into(),
        value: "other@fiction.example".into(),
        kind: "EMAIL".into(),
    })
    .unwrap();
    let ambiguous = rehydrate_checked(&v, "Email_AbCdEf12").unwrap();
    assert_eq!(ambiguous.restored_count, 0);
    assert_eq!(ambiguous.unrestored_tokens.len(), 1);
    let scoped = rehydrate_checked_for_tokens(&v, "EMAIL_abcdef12 email_ABCDEF12", &["EMAIL_abcdef12"]).unwrap();
    assert_eq!(scoped.restored_count, 1);
    assert_eq!(scoped.unrestored_count, 1);
}
#[test]
fn rehydrate_repairs_relabelled_placeholder() {
    let mut v = MemoryVault::default();
    for (token, value, kind) in [
        ("COMPANY_568586ad", "Kestrel Analytics", "COMPANY"),
        ("CUSTOMER_1234abcd", "Hiroshi Tanaka", "CUSTOMER"),
        ("EMAIL_99990000", "x@other.example", "EMAIL"),
    ] {
        v.save_entry(VaultEntry {
            token: token.into(),
            value: value.into(),
            kind: kind.into(),
        })
        .unwrap();
    }
    let got = rehydrate_checked_for_tokens(
        &v,
        "CUSTOMER_568586ad (CUSTOMER_1234abcd) and PERSON_99990000",
        &["COMPANY_568586ad", "CUSTOMER_1234abcd"],
    )
    .unwrap();
    assert_eq!(got.text, "Kestrel Analytics (Hiroshi Tanaka) and PERSON_99990000");
    assert_eq!(got.restored_count, 2);
    assert_eq!(got.unrestored_count, 1);
}
#[test]
fn split_records_carries_csv_header() {
    let input = "name,email\nJordan Avery,jordan@example.com\nJane Smith,jane@example.com\n";
    let parts = split_records(input, 45).unwrap();
    assert!(parts.len() >= 2);
    assert!(parts.iter().all(|t| t.text.starts_with("name,email\n")));
}
#[test]
fn split_records_preserves_long_non_csv_lines() {
    let input = format!("{}\ntrailer\n", "a".repeat(300 * 1024));
    let parts = split_records(&input, 64 * 1024).unwrap();
    assert_eq!(parts.iter().map(|t| t.text.as_str()).collect::<String>(), input);
}
#[test]
fn split_oversized_preserves_utf8() {
    let input = format!("{}\n", "é".repeat(20));
    let parts = split_oversized(&input, 7);
    assert!(parts.len() >= 2);
    assert_eq!(parts.join(""), input);
}
#[test]
fn redact_uses_shared_vault_across_tranches() {
    let mut v = MemoryVault::default();
    let first = redact(&mut v, "Jordan Avery <jordan@example.com>", false).unwrap();
    let second = redact(&mut v, "Reminder for Jordan Avery", false).unwrap();
    let token = &first.entities.iter().find(|e| e.kind == "PERSON").unwrap().token;
    assert!(first.text.contains(token));
    assert!(second.text.contains(token));
}
#[test]
fn value_hash_and_vault_identity() {
    let mut v = MemoryVault::default();
    assert_eq!(
        value_hash(" test "),
        "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08"
    );
    let first = v.ensure_entry("EMAIL", " test ").unwrap();
    let second = v.ensure_entry("PERSON", "test").unwrap();
    assert_eq!(first.token, second.token);
    assert_eq!(first.kind, second.kind);
    assert!(
        regex::Regex::new(r"^EMAIL_[0-9a-f]{8}$")
            .unwrap()
            .is_match(&first.token)
    );
}
#[test]
fn model_callback_error_fallback() {
    let bad = |_: &str| Err::<String, _>(anyhow!("unavailable"));
    assert!(detect_with("a@example.com", &[], Some(&bad), false).is_err());
    assert!(detect_with("a@example.com", &[], Some(&bad), true).unwrap().1);
}
