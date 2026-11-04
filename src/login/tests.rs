// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

use super::*;
use serde_json::json;

fn tty(stdin_tty: bool, stderr_tty: bool, agent: bool, no_input: bool) -> Interactivity {
    Interactivity {
        stdin_tty,
        stderr_tty,
        agent,
        no_input,
    }
}

#[test]
fn login_never_prompts_without_a_terminal_or_in_agent_mode() {
    let human = tty(true, true, false, false);
    assert_eq!(plan_login(false, false, human), LoginPlan::DeviceThenPaste);
    assert_eq!(plan_login(false, true, human), LoginPlan::Paste);
    assert_eq!(plan_login(true, false, human), LoginPlan::Stdin);
    for i in [
        tty(false, true, false, false),
        tty(true, false, false, false),
        tty(true, true, true, false),
        tty(true, true, false, true),
    ] {
        assert_eq!(plan_login(false, false, i), LoginPlan::Refuse, "{i:?}");
        assert_eq!(
            plan_login(true, false, i),
            LoginPlan::Stdin,
            "--with-token works anywhere"
        );
    }
}

#[test]
fn clean_key_trims_and_rejects_junk() {
    assert_eq!(
        clean_key("  sk_mxl_abc123_0123456789abcdef\n").unwrap(),
        "sk_mxl_abc123_0123456789abcdef"
    );
    assert_eq!(
        clean_key("Bearer sk_mxl_abc123_0123456789abcdef").unwrap(),
        "sk_mxl_abc123_0123456789abcdef"
    );
    assert!(clean_key("").is_err());
    assert!(clean_key("two words here, definitely not").is_err());
    assert!(clean_key("short").is_err());
}

#[test]
fn unsupported_statuses_trigger_fallback() {
    for s in [404, 405, 501] {
        assert!(is_unsupported(s));
    }
    for s in [400, 401, 403, 429, 500] {
        assert!(!is_unsupported(s));
    }
}

#[test]
fn device_start_parses_and_only_trusts_mixlayer_hosts() {
    let v = json!({"data": {"device_code": "dc", "user_code": "ABCD-EFGH",
        "verification_uri": "https://console.mixlayer.com/cli/authorize",
        "verification_uri_complete": "https://console.mixlayer.com/cli/authorize?code=ABCD-EFGH",
        "expires_in": 900, "interval": 0}});
    let d = parse_device_start(&v).unwrap();
    assert_eq!(d.user_code, "ABCD-EFGH");
    assert_eq!(d.interval, 1, "interval is clamped");
    assert_eq!(d.expires_in, 900);
    let evil = json!({"device_code": "dc", "user_code": "X", "verification_uri": "https://evil.example/cli"});
    assert!(parse_device_start(&evil).is_none());
    assert!(parse_device_start(&json!({"data": {}})).is_none());
}

#[test]
fn poll_states() {
    assert_eq!(parse_poll(&json!({"data": {"status": "pending"}})), Poll::Pending);
    assert_eq!(parse_poll(&json!({"data": {"status": "slow_down"}})), Poll::SlowDown);
    assert_eq!(parse_poll(&json!({"data": {"status": "denied"}})), Poll::Denied);
    assert_eq!(parse_poll(&json!({"data": {"status": "expired"}})), Poll::Expired);
    assert_eq!(
        parse_poll(&json!({"data": {"status": "approved"}})),
        Poll::Pending,
        "no key yet"
    );
    assert_eq!(
        parse_poll(&json!({"data": {"status": "approved", "api_key": "k"}})),
        Poll::Approved { api_key: "k".into() }
    );
}

#[test]
fn amounts_convert_to_cents_within_limits() {
    assert_eq!(amount_cents("25").unwrap(), 2500);
    assert_eq!(amount_cents("$25.5").unwrap(), 2550);
    assert_eq!(amount_cents("1.00").unwrap(), 100);
    assert_eq!(amount_cents("2000").unwrap(), 200_000);
    for bad in ["0.99", "2000.01", "-5", "abc", "1.234", "", "1e3"] {
        assert!(amount_cents(bad).is_err(), "{bad}");
    }
}

#[test]
fn balance_only_for_prepaid() {
    assert_eq!(
        balance_usd(&json!({"data": {"billing_mode": "prepaid", "credit_balance_cents": 1234}})),
        Some(12.34)
    );
    assert_eq!(
        balance_usd(&json!({"billing_mode": "postpaid", "credit_balance_cents": 1234})),
        None
    );
}

#[test]
fn endpoint_paths_are_encoded() {
    assert_eq!(endpoints::refill("ba_1"), "/v1/billing_accounts/ba_1/refill");
    assert_eq!(
        endpoints::card_setup_session("a/b"),
        "/v1/billing_accounts/a%2Fb/card_setup_session"
    );
    assert!(endpoints::billing_account("ba_1").ends_with("?include=all"));
}
