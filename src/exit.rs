// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

//! Typed exit codes. The numbers are a public contract shared with scripts
//! and agents; they match the Go CLI exactly.

use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Code {
    Success = 0,
    Other = 1,
    Usage = 2,
    NotFound = 3,
    Auth = 4,
    Api = 5,
    Partial = 6,
    RateLimited = 7,
    Billing = 8,
    GateMatched = 9,
    Config = 10,
}

pub const TABLE: &[(Code, &str)] = &[
    (Code::Success, "success"),
    (Code::Other, "other failure"),
    (Code::Usage, "usage error"),
    (Code::NotFound, "not found (HTTP 404)"),
    (Code::Auth, "authentication or permission (HTTP 401/403, some HTTP 400)"),
    (Code::Api, "API failure (including other HTTP errors)"),
    (Code::Partial, "partial failure in a successful response"),
    (Code::RateLimited, "rate limited (HTTP 429; honor Retry-After)"),
    (
        Code::Billing,
        "billing or insufficient prepaid credits (HTTP 402; do not retry)",
    ),
    (
        Code::GateMatched,
        "decision gate matched (decide --fail-if; response printed)",
    ),
    (Code::Config, "configuration failure"),
];

/// An error that carries its exit code. Anything else maps to [`Code::Other`].
#[derive(Debug)]
pub struct CliError {
    pub code: Code,
    pub message: String,
}

/// Exit status from a child command run by `keys mint`.
#[derive(Debug)]
pub struct ChildExit(pub u8);

impl fmt::Display for ChildExit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "child exited with status {}", self.0)
    }
}

impl std::error::Error for ChildExit {}

pub fn status_of(e: &anyhow::Error) -> u8 {
    e.chain()
        .find_map(|c| c.downcast_ref::<ChildExit>())
        .map(|e| e.0)
        .unwrap_or(code_of(e) as u8)
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CliError {}

pub fn err(code: Code, message: impl Into<String>) -> anyhow::Error {
    CliError {
        code,
        message: message.into(),
    }
    .into()
}

pub fn usage(message: impl Into<String>) -> anyhow::Error {
    err(Code::Usage, message)
}

/// Exit code for any error returned by a command.
pub fn code_of(e: &anyhow::Error) -> Code {
    e.chain()
        .find_map(|c| c.downcast_ref::<CliError>())
        .map(|c| c.code)
        .unwrap_or(Code::Other)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_stable() {
        for (i, (code, _)) in TABLE.iter().enumerate() {
            assert_eq!(*code as usize, i);
        }
    }

    #[test]
    fn code_survives_context() {
        let e = usage("bad flag").context("while parsing");
        assert_eq!(code_of(&e), Code::Usage);
        assert_eq!(code_of(&anyhow::anyhow!("plain")), Code::Other);
    }
}
