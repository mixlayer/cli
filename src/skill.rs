// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

use crate::spec_types::Cmd;

pub const TEXT: &str = include_str!("../skills/mixlayer/SKILL.md");
pub const PRIVACY: &str = include_str!("../skills/mixlayer/references/privacy.md");
pub const COMMANDS: &str = include_str!("../skills/mixlayer/references/commands.md");

pub fn files() -> [(&'static str, &'static str); 3] {
    [
        ("SKILL.md", TEXT),
        ("references/privacy.md", PRIVACY),
        ("references/commands.md", COMMANDS),
    ]
}

/// Render the checked-in reference from the declarative command table.
pub fn generated_command_reference() -> String {
    let mut out = String::from(
        "# Mixlayer commands\n\nGenerated from the Cobra command tree by `go generate ./skills/mixlayer`. Do not edit manually. Use `--help` for detailed syntax.\n\n",
    );
    fn visit<'a>(prefix: &str, cmds: &'a [Cmd], out: &mut Vec<(String, &'a Cmd)>) {
        for cmd in cmds {
            if cmd.hidden || cmd.experimental || cmd.name == "completion" {
                continue;
            }
            let path = format!("{prefix}{}", cmd.name);
            if path != "agent" && path != "agent skill" {
                out.push((path.clone(), cmd));
            }
            visit(&format!("{path} "), cmd.subcommands, out);
        }
    }
    let mut paths = Vec::new();
    visit("", crate::spec::COMMANDS, &mut paths);
    paths.push((
        "agent-context".into(),
        crate::spec_types::find("agent-context").unwrap_or_else(|| {
            // `agent-context` is constructed in cli.rs, outside the generated spec.
            static CONTEXT: Cmd = Cmd {
                name: "agent-context",
                usage: "agent-context",
                short: "Emit structured JSON describing this CLI for agents",
                annotations: &[],
                experimental: false,
                hidden: false,
                flags: &[],
                subcommands: &[],
            };
            &CONTEXT
        }),
    ));
    paths.sort_by(|a, b| a.0.cmp(&b.0));
    for (path, cmd) in paths {
        let mut flags: Vec<String> = crate::cli::GLOBAL_FLAGS
            .iter()
            .map(|(name, _, _, _)| format!("--{name}"))
            .collect();
        flags.extend(
            cmd.flags
                .iter()
                .filter(|f| !crate::spec_types::flag_presentation(&path, f, false).1)
                .map(|f| format!("--{}", f.name)),
        );
        // Cobra exposes the shared database flag on these command groups and
        // their children; the flat agent-context table stores only local flags.
        if path == "shield dict" || path.starts_with("shield dict ") || path == "vault" || path.starts_with("vault ") {
            flags.push("--db".into());
        }
        if path == "agent-context" {
            flags.push("--pretty".into());
        }
        flags.sort();
        flags.dedup();
        out.push_str(&format!(
            "## `mixlayer {path}`\n\n{}\n\nFlags: {}\n\n",
            cmd.short,
            flags.join(", ")
        ));
    }
    format!("{}\n", out.trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;
    use regex::Regex;
    #[test]
    fn generated_reference_is_current() {
        assert_eq!(COMMANDS, generated_command_reference());
    }

    #[test]
    fn documented_commands_exist_in_tree() {
        let root = crate::cli::build();
        let inline = Regex::new(r"`(mixlayer [^`]+)`").unwrap();
        let mut count = 0;
        let mut documents = vec![TEXT, PRIVACY, COMMANDS];
        let readme = include_str!("../README.md");
        documents.push(readme);
        for doc in documents {
            for line in doc.lines() {
                let line = line.trim();
                let mut examples: Vec<&str> = inline.captures_iter(line).map(|c| c.get(1).unwrap().as_str()).collect();
                if line.starts_with("mixlayer ") {
                    examples.push(line);
                }
                for example in examples {
                    let mut command = root.clone();
                    let mut found = false;
                    for word in example.split_whitespace().skip(1) {
                        if word.starts_with('-') {
                            break;
                        }
                        let Some(child) = command.find_subcommand(word) else {
                            break;
                        };
                        command = child.clone();
                        found = true;
                    }
                    assert!(found, "undocumented command in {example:?}");
                    for word in example.split_whitespace().filter(|w| w.starts_with("--") && *w != "--") {
                        let name = word.trim_start_matches("--").split('=').next().unwrap();
                        let exists = command
                            .get_arguments()
                            .chain(root.get_arguments())
                            .any(|arg| arg.get_long() == Some(name));
                        assert!(exists, "unknown flag --{name} in {example:?}");
                    }
                    count += 1;
                }
            }
        }
        assert!(count >= 60, "only {count} documented commands checked");
    }

    #[test]
    fn embedded_skill_frontmatter() {
        assert!(TEXT.starts_with("---\nname: mixlayer\ndescription: "));
        assert!(TEXT.contains("Never print vault values"));
        assert!(TEXT.len() > 2000);
        let lines = TEXT.matches('\n').count() + 1;
        assert!(lines < 150, "skill has {lines} lines, want fewer than 150");
        let front = TEXT.split("---").nth(1).unwrap();
        for trigger in ["OpenAI-compatible", "Claude", "GPT", "prices", "redaction", "credits"] {
            assert!(front.contains(trigger), "frontmatter description missing {trigger:?}");
        }
    }
}
