// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

use std::io::IsTerminal;
use std::process::ExitCode;

use clap::ArgMatches;

use mixlayer::exit::Code;
use mixlayer::{
    Globals, agent_context, agent_detected, agent_setup, api, cli, exit, ledger, output, paths, profiles, shield_cmd,
    skill, spec_types, which,
};

fn flag(m: &ArgMatches, name: &str) -> Option<bool> {
    match m.value_source(name) {
        Some(clap::parser::ValueSource::CommandLine) => m.try_get_one::<bool>(name).ok().flatten().copied(),
        _ => None,
    }
}

fn globals(m: &ArgMatches) -> Globals {
    let explicit_human = flag(m, "human-friendly") == Some(true);
    let agent_mode = flag(m, "agent") == Some(true) || (agent_detected() && !explicit_human);
    let pick = |name: &str| flag(m, name).unwrap_or(agent_mode);
    Globals {
        agent: flag(m, "agent") == Some(true),
        json: pick("json"),
        compact: pick("compact"),
        no_input: pick("no-input"),
        no_color: pick("no-color"),
        yes: pick("yes"),
        dry_run: flag(m, "dry-run") == Some(true),
        select: m.get_one::<String>("select").cloned(),
    }
}

/// Global flags are declared on the root and propagated, so the deepest
/// subcommand's matches hold their values.
fn leaf_matches(m: &ArgMatches) -> (Vec<&str>, &ArgMatches) {
    let mut path = Vec::new();
    let mut leaf = m;
    while let Some((n, s)) = leaf.subcommand() {
        path.push(n);
        leaf = s;
    }
    (path, leaf)
}

fn main() -> ExitCode {
    output::quiet_broken_pipe();
    let argv: Vec<std::ffi::OsString> = std::env::args_os().collect();
    let matches = match parse(&argv) {
        Ok(m) => m,
        Err(e) => {
            let code = match e.kind() {
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion => Code::Success,
                _ => Code::Usage,
            };
            let _ = e.print();
            return ExitCode::from(code as u8);
        }
    };
    let matches = match apply_profile(&argv, matches) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("Error: {e:#}");
            return ExitCode::from(exit::code_of(&e) as u8);
        }
    };
    match run(&matches) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Error: {e:#}");
            ExitCode::from(exit::code_of(&e) as u8)
        }
    }
}

fn parse(argv: &[std::ffi::OsString]) -> Result<ArgMatches, clap::Error> {
    cli::build().try_get_matches_from(argv)
}

/// `--profile <name>`: overlay the profile's values onto flags the user did
/// not set, then parse again so every command sees them as ordinary flags.
fn apply_profile(argv: &[std::ffi::OsString], m: ArgMatches) -> anyhow::Result<ArgMatches> {
    let (path, leaf) = leaf_matches(&m);
    let Some(name) = leaf.get_one::<String>("profile").filter(|n| !n.is_empty()) else {
        return Ok(m);
    };
    if let Some(home) = leaf.get_one::<String>("home") {
        paths::set_home_override(home).map_err(|e| exit::usage(e.to_string()))?;
    }
    let profile = profiles::resolve(name)?;
    let mut root = cli::build();
    root.build();
    let mut cmd = &root;
    for p in &path {
        cmd = cmd.find_subcommand(p).expect("path came from clap");
    }
    let extra = profiles::overlay_args(&profile, cmd, leaf);
    if extra.is_empty() {
        return Ok(m);
    }
    // Insert right after the last subcommand word so leaf flags are accepted
    // and trailing positionals are not disturbed.
    let mut at = 1;
    let mut want = path.iter();
    let mut next = want.next();
    for (i, a) in argv.iter().enumerate().skip(1) {
        match next {
            Some(w) if a.to_str() == Some(*w) => {
                at = i + 1;
                next = want.next();
            }
            None => break,
            _ => {}
        }
    }
    let mut args: Vec<std::ffi::OsString> = argv[..at].to_vec();
    args.extend(extra.into_iter().map(Into::into));
    args.extend(argv[at..].iter().cloned());
    parse(&args).map_err(|e| exit::usage(format!("applying profile {name:?}: {}", e.kind())))
}

fn run(m: &ArgMatches) -> anyhow::Result<()> {
    let (path, leaf) = leaf_matches(m);
    if let Some(home) = leaf.get_one::<String>("home") {
        paths::set_home_override(home).map_err(|e| exit::usage(e.to_string()))?;
    }
    let g = globals(leaf);
    if path.is_empty() {
        cli::build().print_long_help()?;
        return Ok(());
    }
    match path.as_slice() {
        ["agent", "setup"] => {
            let name = leaf.get_one::<String>("agent").cloned().unwrap_or_else(|| "all".into());
            let explicit_yes = flag(leaf, "yes") == Some(true);
            let report = agent_setup::run(agent_setup::Options {
                agent: name,
                mcp_binary: agent_setup::mcp_binary(),
                dry_run: g.dry_run || !explicit_yes,
                uninstall: leaf
                    .try_get_one::<bool>("uninstall")
                    .ok()
                    .flatten()
                    .copied()
                    .unwrap_or(false),
            })?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        ["agent", "skill", "print"] => output::write_stdout(skill::TEXT.as_bytes()),
        ["agent", "skill", "path"] => {
            use sha2::{Digest, Sha256};
            let hash = Sha256::digest(skill::TEXT.as_bytes());
            let home = paths::user_home().ok_or_else(|| anyhow::anyhow!("resolve user cache dir: not found"))?;
            let cache = if cfg!(target_os = "macos") {
                home.join("Library/Caches")
            } else {
                std::env::var_os("XDG_CACHE_HOME")
                    .map(std::path::PathBuf::from)
                    .unwrap_or_else(|| home.join(".cache"))
            };
            let path = cache
                .join("mixlayer")
                .join(format!("skill-{}", hex::encode(&hash[..8])));
            agent_setup::materialize_skill(&path)?;
            println!("{}", path.display());
            Ok(())
        }
        ["agent", "skill-ref"] => output::write_stdout(skill::generated_command_reference().as_bytes()),
        ["version"] => {
            if g.json || g.agent {
                output::print_json(&serde_json::json!({"name": "mixlayer", "version": cli::VERSION}), &g)
            } else {
                println!("mixlayer {}", cli::VERSION);
                Ok(())
            }
        }
        ["agent-context"] => {
            let v = agent_context::build();
            let s = if leaf.get_flag("pretty") {
                serde_json::to_string_pretty(&v)?
            } else {
                serde_json::to_string(&v)?
            };
            println!("{s}");
            Ok(())
        }
        ["which"] => run_which(leaf, &g),
        ["completion", shell] => {
            let shell = match *shell {
                "bash" => clap_complete::Shell::Bash,
                "zsh" => clap_complete::Shell::Zsh,
                "fish" => clap_complete::Shell::Fish,
                "powershell" => clap_complete::Shell::PowerShell,
                other => return Err(exit::usage(format!("unsupported shell {other:?}"))),
            };
            let mut buf = Vec::new();
            clap_complete::generate(shell, &mut cli::build(), "mixlayer", &mut buf);
            output::write_stdout(&buf)
        }
        _ => {
            if let Some(result) = shield_cmd::dispatch(&path, leaf, &g) {
                return result;
            }
            if let Some(result) = api::commands::dispatch(&path, leaf, &g) {
                return result;
            }
            if let Some(result) = ledger::dispatch(&path, leaf, &g) {
                return result;
            }
            if let Some(result) = profiles::dispatch(&path, leaf, &g) {
                return result;
            }
            let full = path.join(" ");
            if spec_types::find(&full).is_some_and(|c| !c.subcommands.is_empty()) {
                // A group with no subcommand prints its help, like cobra.
                let mut root = cli::build();
                root.build();
                let mut c = &mut root;
                for p in &path {
                    c = c.find_subcommand_mut(p).expect("path came from clap");
                }
                c.print_help()?;
                return Ok(());
            }
            Err(exit::err(
                Code::Other,
                format!("`mixlayer {full}` is not available in the Rust build yet"),
            ))
        }
    }
}

fn run_which(m: &ArgMatches, g: &Globals) -> anyhow::Result<()> {
    let limit: usize = m
        .get_one::<String>("limit")
        .map(|s| s.parse())
        .transpose()
        .map_err(|e| exit::usage(format!("invalid --limit: {e}")))?
        .unwrap_or(3);
    let query = cli::positional_words(spec_types::find("which").expect("which is in the table"), m).join(" ");
    let index = which::active_index(cli::jini_enabled());
    let as_json = g.json || !std::io::stdout().is_terminal();
    if query.trim().is_empty() {
        return render_which(which::rank(&index, "", limit), as_json, g);
    }
    let matches = which::rank(&index, &query, limit);
    if matches.is_empty() {
        if g.json {
            return output::print_json(&serde_json::json!({"matches": []}), g);
        }
        return Err(exit::usage(format!(
            "no match for {query:?}; try 'mixlayer --help' for the full command list"
        )));
    }
    render_which(matches, as_json, g)
}

fn render_which(matches: Vec<which::Match>, as_json: bool, g: &Globals) -> anyhow::Result<()> {
    if as_json {
        return output::print_json(&serde_json::json!({"matches": matches}), g);
    }
    println!("{:<24}  {:<8}  DESCRIPTION", "COMMAND", "SCORE");
    for m in matches {
        println!("{:<24}  {:<8}  {}", m.entry.command, m.score, m.entry.description);
    }
    Ok(())
}
