use std::io::{IsTerminal, Read};

use clap::ArgMatches;

use super::{arg, bool_arg, print, words};
use crate::Globals;
use crate::api::{config, console_url};
use crate::exit::{self, Code};

pub fn setup(m: &ArgMatches) -> anyhow::Result<()> {
    let signup = console_url("/sign-up", "auth-setup");
    let keys = console_url("/app/api-keys", "auth-setup");
    println!("1. Sign up at {signup}");
    println!("   Then create an API key at {keys}");
    println!("2. Put it in your environment:");
    println!("     export MIXLAYER_API_KEY=\"your-key\"");
    println!("3. Check it works:");
    println!("     mixlayer doctor");
    if bool_arg(m, "launch") {
        #[cfg(target_os = "macos")]
        let mut cmd = std::process::Command::new("open");
        #[cfg(windows)]
        let mut cmd = {
            let mut c = std::process::Command::new("rundll32");
            c.arg("url.dll,FileProtocolHandler");
            c
        };
        #[cfg(not(any(target_os = "macos", windows)))]
        let mut cmd = std::process::Command::new("xdg-open");
        if let Err(e) = cmd.arg(&signup).spawn() {
            eprintln!("could not open a browser: {e}");
        }
    }
    Ok(())
}

pub fn set_token(m: &ArgMatches, g: &Globals) -> anyhow::Result<()> {
    let args = words(m, "auth set-token");
    if args.len() > 1 {
        return Err(exit::usage("set-token accepts at most one token"));
    }
    let token = if let Some(value) = args.first().filter(|s| s.as_str() != "-") {
        value.trim().to_owned()
    } else {
        if std::io::stdin().is_terminal() {
            return Err(exit::usage(
                "pipe the key on stdin, for example: pbpaste | mixlayer auth set-token",
            ));
        }
        let mut bytes = Vec::new();
        std::io::stdin().take(64 * 1024).read_to_end(&mut bytes)?;
        String::from_utf8(bytes)
            .map_err(|_| exit::usage("API key must be UTF-8"))?
            .trim()
            .to_owned()
    };
    if token.is_empty() || token.chars().any(char::is_whitespace) {
        return Err(exit::usage("expected a single API key with no spaces"));
    }
    let cfg = config::load(arg(m, "config")).map_err(|e| exit::err(Code::Config, e.to_string()))?;
    cfg.save_token(&token)
        .map_err(|e| exit::err(Code::Config, format!("saving token: {e}")))?;
    if g.json {
        print(
            &serde_json::json!({"saved": true, "config_path": cfg.path, "credentials_path": cfg.credentials_path}),
            g,
        )
    } else {
        println!("Token saved to {}", cfg.credentials_path.display());
        Ok(())
    }
}

pub fn status(m: &ArgMatches, g: &Globals) -> anyhow::Result<()> {
    let cfg = config::load(arg(m, "config")).map_err(|e| exit::err(Code::Config, e.to_string()))?;
    let present = !cfg.auth_header.is_empty();
    if g.json {
        print(
            &serde_json::json!({"authenticated": present, "verified": false, "source": cfg.auth_source, "config": cfg.path}),
            g,
        )?;
    } else if present {
        println!(
            "Credentials present (not verified)\n  Source: {}\n  Config: {}",
            cfg.auth_source,
            cfg.path.display()
        );
    } else {
        println!(
            "Not authenticated\n\nSet your API key:\n  export MIXLAYER_API_KEY=\"your-key\"\nCreate one at {}",
            console_url("/sign-up", "auth-status")
        );
    }
    if present {
        Ok(())
    } else {
        Err(exit::err(Code::Auth, "no credentials configured"))
    }
}

pub fn logout(m: &ArgMatches, g: &Globals) -> anyhow::Result<()> {
    let cfg = config::load(arg(m, "config")).map_err(|e| exit::err(Code::Config, e.to_string()))?;
    cfg.logout()
        .map_err(|e| exit::err(Code::Config, format!("clearing tokens: {e}")))?;
    let env = std::env::var("MIXLAYER_API_KEY").ok().filter(|s| !s.is_empty());
    if g.json {
        let mut value = serde_json::json!({"cleared": true});
        if env.is_some() {
            value["note"] = "MIXLAYER_API_KEY env var is still set".into();
        }
        print(&value, g)
    } else {
        if env.is_some() {
            println!("Config cleared. Note: MIXLAYER_API_KEY env var is still set.");
        } else {
            println!("Logged out. Credentials cleared.");
        }
        Ok(())
    }
}
