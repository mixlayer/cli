// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

use super::*;
use std::path::Path;

fn ctx(interactive: bool, agent: bool, no_input: bool) -> Context {
    Context {
        interactive,
        agent,
        no_input,
    }
}

#[test]
fn mode_defaults_to_ask_and_env_wins() {
    assert_eq!(resolve_mode(None, None), Mode::Ask);
    assert_eq!(resolve_mode(None, Some("never")), Mode::Never);
    assert_eq!(resolve_mode(Some("always"), Some("never")), Mode::Always);
    assert_eq!(resolve_mode(Some("0"), None), Mode::Never);
    assert_eq!(resolve_mode(Some("bogus"), Some("always")), Mode::Always);
    assert_eq!(resolve_mode(Some("bogus"), Some("bogus")), Mode::Ask);
}

#[test]
fn consent_never_prompts_without_tty_or_in_agent_mode() {
    for mode in [Mode::Ask, Mode::Always, Mode::Never] {
        assert_eq!(decide_after_error(mode, ctx(false, false, false)), Offer::Skip);
        assert_eq!(decide_after_error(mode, ctx(true, true, false)), Offer::Skip);
    }
    assert_eq!(decide_after_error(Mode::Ask, ctx(true, false, false)), Offer::Prompt);
    assert_eq!(decide_after_error(Mode::Ask, ctx(true, false, true)), Offer::Skip);
    assert_eq!(decide_after_error(Mode::Always, ctx(true, false, false)), Offer::Show);
    assert_eq!(decide_after_error(Mode::Never, ctx(true, false, false)), Offer::Skip);
}

#[test]
fn scrub_removes_secrets_paths_emails_hosts() {
    let home = Path::new("/Users/alice");
    let raw = "key sk-abcdef1234567890XYZ and Bearer abcdefghijklmnop and MIXLAYER_API_KEY=hunter2hunter2 \
               file /Users/alice/proj/secret.txt or /etc/hosts or C:\\Users\\bob\\x.txt \
               mail alice@example.com host db1.corp ip 10.0.0.12 \
               url https://internal.example.net/x?y=1 keep https://github.com/mixlayer/cli/issues/1 \
               blob 0123456789abcdef0123456789abcdef0123";
    let s = scrub(raw, Some(home));
    for leak in [
        "sk-abcdef",
        "abcdefghijklmnop",
        "hunter2",
        "alice",
        "secret.txt",
        "/etc/hosts",
        "bob",
        "example.com",
        "db1.corp",
        "10.0.0.12",
        "internal.example.net",
        "0123456789abcdef",
    ] {
        assert!(!s.contains(leak), "{leak:?} leaked in {s:?}");
    }
    assert!(s.contains("https://github.com/mixlayer/cli/issues/1"), "{s}");
    assert!(
        s.contains("[secret]") && s.contains("[path]") && s.contains("[email]"),
        "{s}"
    );
    assert!(s.contains("MIXLAYER_API_KEY=[secret]"), "{s}");
}

#[test]
fn scrub_caps_length_and_keeps_plain_text() {
    assert_eq!(scrub("  compare is confusing  ", None), "compare is confusing");
    let long = "word ".repeat(300);
    let s = scrub(&long, None);
    assert_eq!(s.chars().count(), MAX_NOTE, "ellipsis counts toward the cap");
    assert!(s.ends_with('…'));
}

#[test]
fn scrub_removes_whole_paths_with_spaces() {
    for (raw, gone) in [
        (r"see C:\Work\Jane Doe\secret.txt now", ["Jane", "Doe", "secret"]),
        ("see /tmp/Jane Doe/secret.txt now", ["Jane", "Doe", "secret"]),
    ] {
        let s = scrub(raw, None);
        for leak in gone {
            assert!(!s.contains(leak), "{leak:?} leaked in {s:?}");
        }
        assert_eq!(s, "see [path] now");
    }
}

#[test]
fn clean_command_drops_positionals() {
    assert_eq!(clean_command(&["models", "list"]), "models list");
    assert_eq!(clean_command(&["ask", "What is my SSN 123?"]), "ask");
    assert_eq!(clean_command(&["shield", "scan", "/tmp/x"]), "shield scan");
}

#[test]
fn clean_command_keeps_only_real_command_names() {
    // `--command 'ask private customer'` must not carry free text into a report.
    assert_eq!(clean_command(&["ask", "private", "customer"]), "ask");
    assert_eq!(clean_command(&["acme", "secret"]), "");
    assert_eq!(clean_command(&["shield", "dict", "add"]), "shield dict add");
}

#[test]
fn config_mode_reads_the_selected_config_file() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("custom.toml");
    std::fs::write(&p, "[feedback]\nmode = \"never\"\n").unwrap();
    assert_eq!(config_mode(p.to_str()).as_deref(), Some("never"));
}

#[test]
fn fingerprint_is_stable_and_ignores_the_note() {
    let mut a = Report::new(Kind::Error, "ask".into(), "0.3.1");
    a.error_kind = Some("authentication-or-permission".into());
    let mut b = a.clone();
    b.note = Some("different words".into());
    assert_eq!(a.fingerprint(), b.fingerprint());
    assert_eq!(
        a.fingerprint(),
        fingerprint("ask", "authentication-or-permission", "0.3.1")
    );
    assert!(a.fingerprint().starts_with("mxl-") && a.fingerprint().len() == 16);
    let mut c = a.clone();
    c.version = "0.3.2".into();
    assert_ne!(a.fingerprint(), c.fingerprint());
    let mut d = a.clone();
    d.command = "compare".into();
    assert_ne!(a.fingerprint(), d.fingerprint());
}

#[test]
fn labels_cover_kind_command_exit_version_os() {
    let mut r = Report::new(Kind::Error, "models list".into(), "0.3.1");
    r.automatic = true;
    r.exit_code = Some(4);
    r.error_kind = Some(exit_kind(4));
    let l = r.labels();
    for want in [
        "feedback",
        "automated",
        "auto",
        "kind:error",
        "cmd:models-list",
        "exit:4",
        "v0.3.1",
    ] {
        assert!(l.iter().any(|x| x == want), "missing {want} in {l:?}");
    }
    assert!(l.iter().any(|x| x.starts_with("os:")));
    assert_eq!(exit_kind(4), "authentication-or-permission");
    assert_eq!(exit_kind(200), "other-failure");
}

#[test]
fn issue_url_is_prefilled_and_encoded() {
    let mut r = Report::new(Kind::Missing, "compare".into(), "0.3.1");
    r.note = Some("needs a & b = c".into());
    let u = url::Url::parse(&issue_url(&r)).unwrap();
    assert_eq!(u.host_str(), Some("github.com"));
    assert_eq!(u.path(), "/mixlayer/cli/issues/new");
    let q: std::collections::HashMap<_, _> = u.query_pairs().into_owned().collect();
    assert!(q["title"].starts_with("[mxl-feedback] [missing] mxl compare: needs a & b = c"));
    assert!(q["body"].contains(&r.fingerprint()));
    assert!(q["body"].contains("needs a & b = c"));
    for want in ["feedback", "automated", "user-report"] {
        assert!(q["labels"].split(',').any(|l| l == want), "{want}");
    }
    let s = url::Url::parse(&search_url("mxl-abc")).unwrap();
    assert!(s.query_pairs().any(|(k, v)| k == "q" && v.contains("\"mxl-abc\"")));
}

#[test]
fn body_never_contains_unscrubbed_fields() {
    let r = Report::new(Kind::Bug, "ask".into(), "0.3.1");
    let b = r.body();
    assert!(b.contains("| command | `mxl ask` |"));
    assert!(!b.to_lowercase().contains("api_key"));
}

#[test]
fn rate_limits_per_fingerprint_and_per_day() {
    let mut l = Ledger::default();
    let t = 1_000_000;
    assert!(l.allowed("a", t, "2026-11-03"));
    l.mark_offered("a", t, "2026-11-03");
    assert!(!l.allowed("a", t + 60, "2026-11-03"));
    assert!(l.allowed("a", t + PER_FINGERPRINT_SECS, "2026-11-04"));
    for i in 0..PER_DAY - 1 {
        l.mark_offered(&format!("x{i}"), t, "2026-11-03");
    }
    assert!(!l.allowed("fresh", t, "2026-11-03"), "daily budget");
    assert!(l.allowed("fresh", t, "2026-11-04"), "new day resets");
}

#[test]
fn retries_are_counted_inside_the_window() {
    let mut l = Ledger::default();
    assert_eq!(l.record_failure("a", 0), 1);
    assert_eq!(l.record_failure("a", 60), 2);
    assert_eq!(l.record_failure("a", 60 + RETRY_WINDOW_SECS), 1);
}

#[test]
fn first_repeat_is_offered_despite_the_fingerprint_cooldown() {
    let mut l = Ledger::default();
    let t = 1_000_000;
    let day = "2026-11-03";
    assert_eq!(l.record_failure("a", t), 1);
    assert!(l.should_offer("a", 1, t, day));
    l.mark_offered("a", t, day);
    // Same failure again inside the retry window: friction gets its own offer.
    assert_eq!(l.record_failure("a", t + 60), 2);
    assert!(l.should_offer("a", 2, t + 60, day));
    l.mark_offered("a", t + 60, day);
    // Further repeats are back under the 24-hour rule.
    assert_eq!(l.record_failure("a", t + 120), 3);
    assert!(!l.should_offer("a", 3, t + 120, day));
    // The daily budget still caps friction offers.
    for i in 0..PER_DAY {
        l.mark_offered(&format!("x{i}"), t, day);
    }
    assert!(!l.should_offer("b", 2, t, day));
}

#[test]
fn ledger_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("feedback.json");
    let mut l = Ledger::default();
    l.mark_offered("a", 5, "2026-11-03");
    l.record_failure("a", 5);
    {
        let _lock = Ledger::lock(&p).expect("lock");
        l.save(&p);
    }
    assert_eq!(Ledger::load(&p), l);
    assert!(!p.with_extension("json.tmp").exists(), "temp file renamed into place");
    assert_eq!(Ledger::load(&dir.path().join("missing.json")), Ledger::default());
}
