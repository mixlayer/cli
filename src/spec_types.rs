// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

//! Types for the declarative command table in `spec.rs`.

/// Value type of a flag. The names returned by [`Kind::go_name`] match the
/// Go CLI's pflag type names, so `agent-context` output is unchanged for
/// agents that already parse it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Str,
    Bool,
    Int,
    Int64,
    Float,
    Duration,
    /// Comma-separated values, repeatable (`--x a,b --x c`).
    StrSlice,
    /// Repeatable, no comma splitting.
    StrArray,
}

impl Kind {
    pub fn go_name(self) -> &'static str {
        match self {
            Kind::Str => "string",
            Kind::Bool => "bool",
            Kind::Int => "int",
            Kind::Int64 => "int64",
            Kind::Float => "float64",
            Kind::Duration => "duration",
            Kind::StrSlice => "stringSlice",
            Kind::StrArray => "stringArray",
        }
    }
}

#[derive(Debug)]
pub struct Flag {
    pub name: &'static str,
    pub short: Option<char>,
    pub kind: Kind,
    pub usage: &'static str,
    /// Default as shown to users and agents (Go's DefValue string).
    pub default: Option<&'static str>,
}

#[derive(Debug)]
pub struct Cmd {
    pub name: &'static str,
    /// Usage line, e.g. `ask <question>`. Positional arguments are parsed
    /// from it: `<x>` required, `[x]` optional, a trailing `...` repeats.
    pub usage: &'static str,
    pub short: &'static str,
    pub annotations: &'static [(&'static str, &'static str)],
    /// Registered only when `MIXLAYER_ENABLE_JINI=1`.
    pub experimental: bool,
    /// Left out of `--help` (still callable and still in agent-context).
    pub hidden: bool,
    pub flags: &'static [Flag],
    pub subcommands: &'static [Cmd],
}

impl Cmd {
    /// Positional arguments declared in the usage line.
    pub fn positionals(&self) -> Vec<Positional> {
        self.usage
            .split_whitespace()
            .skip(1)
            // `[--<flag> <value> ...]` documents trailing flags, not a positional.
            .take_while(|tok| !tok.starts_with("[--"))
            .filter_map(|tok| {
                let (required, inner) = if let Some(t) = tok.strip_prefix('<') {
                    (true, t.trim_end_matches("...").trim_end_matches('>'))
                } else {
                    let t = tok.strip_prefix('[')?;
                    (false, t.trim_end_matches("...").trim_end_matches(']'))
                };
                let inner = inner.trim_end_matches("...").trim_end_matches(['>', ']']);
                Some(Positional {
                    name: inner.to_string(),
                    required,
                    variadic: tok.contains("..."),
                })
            })
            .collect()
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Positional {
    pub name: String,
    pub required: bool,
    pub variadic: bool,
}

/// Flags whose presentation depends on `MIXLAYER_ENABLE_JINI`. The table in
/// `spec.rs` holds the Jini-on usage. With Jini off, a `Some` replaces the
/// usage text and a `None` hides the flag from `--help` (agent-context
/// still lists it, as in the Go CLI).
pub const JINI_OFF_FLAGS: &[(&str, &str, Option<&str>)] = &[
    ("escalate", "checker", Some("Answer checker")),
    ("escalate", "checker-model", None),
    ("escalate", "threshold", None),
];

/// (usage, hidden_in_help) for a flag, given whether Jini is enabled.
pub fn flag_presentation(cmd_path: &str, flag: &Flag, jini: bool) -> (&'static str, bool) {
    if !jini
        && let Some((_, _, over)) = JINI_OFF_FLAGS
            .iter()
            .find(|(p, n, _)| *p == cmd_path && *n == flag.name)
    {
        return match over {
            Some(u) => (u, false),
            None => (flag.usage, true),
        };
    }
    (flag.usage, false)
}

/// Finds a command by its space-separated path below the root, e.g. `shield ask`.
pub fn find(path: &str) -> Option<&'static Cmd> {
    let mut level: &'static [Cmd] = crate::spec::COMMANDS;
    let mut found = None;
    for part in path.split_whitespace() {
        let cmd = level.iter().find(|c| c.name == part)?;
        level = cmd.subcommands;
        found = Some(cmd);
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positionals_from_usage() {
        let c = Cmd {
            name: "x",
            usage: "x <question> [file] [rest...]",
            short: "",
            annotations: &[],
            experimental: false,
            hidden: false,
            flags: &[],
            subcommands: &[],
        };
        let p = c.positionals();
        assert_eq!(p.len(), 3);
        assert!(p[0].required && !p[0].variadic && p[0].name == "question");
        assert!(!p[1].required && p[1].name == "file");
        assert!(p[2].variadic && p[2].name == "rest");
    }

    #[test]
    fn find_nested() {
        assert_eq!(find("shield ask").map(|c| c.name), Some("ask"));
        assert!(find("shield nope").is_none());
    }
}
