mod commands;
mod decision;
mod human;

use crate::Globals;
use clap::ArgMatches;

pub fn dispatch(path: &[&str], m: &ArgMatches, g: &Globals) -> Option<anyhow::Result<()>> {
    if let [name] = path
        && ["ladder", "escalate", "council", "compare", "grep", "replay", "sql"].contains(name)
        && crate::cli::positional_words(crate::spec_types::find(name).expect("command"), m).is_empty()
    {
        return Some((|| {
            let mut root = crate::cli::build();
            root.find_subcommand_mut(name).expect("command").print_help()?;
            Ok(())
        })());
    }
    Some(match path {
        ["ladder"] => commands::ladder(m, g),
        ["escalate"] => commands::escalate(m, g),
        ["council"] => commands::council(m, g),
        ["compare"] => commands::compare(m, g),
        ["savings"] => commands::savings(m, g),
        ["grep"] => commands::grep(m, g),
        ["replay"] => commands::replay(m, g),
        ["sql"] => commands::sql(m, g),
        ["decide"] => decision::decide(m, g),
        ["decide", "batch"] => decision::batch(m, g),
        _ => return None,
    })
}
