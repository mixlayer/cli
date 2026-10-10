// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

//! Long help and examples for commands added after the Go port. Maintained by
//! hand: `help_text.rs` is regenerated from the Go snapshots, so entries there
//! would be overwritten. Same shape: (path, long, example).

pub const HELP: &[(&str, &str, &str)] = &[
    (
        r#"agent setup"#,
        r#"Install the Mixlayer Agent Skill and the local mixlayer-mcp server into
Claude Code, Codex, Cursor, Hermes or Claude Desktop. Without --yes this only
previews the changes.

--platform also registers Mixlayer's hosted Platform MCP server
(https://api.mixlayer.com/v1/mcp) as "mixlayer-platform". Its tools manage
organizations, models, members, API keys, analytics and billing, limited by
the permissions of the key the agent uses. The agent config stores only the name
of the environment variable that holds the key (--key-env, default
MIXLAYER_API_KEY), never the key itself. If that variable is set, setup checks
the key with one read-only Platform API request and reports the result.
Supported for Claude Code, Codex and Cursor; Hermes and Claude Desktop are
reported as unsupported."#,
        r#"  mixlayer agent setup --agent claude-code
  mixlayer agent setup --agent all --yes
  mixlayer agent setup --agent claude-code --platform --yes
  mixlayer agent setup --agent codex --platform --key-env MIXLAYER_PLATFORM_KEY --yes
  mixlayer agent setup --agent all --uninstall --yes"#,
    ),
    (
        r#"keys"#,
        r#"Manage your organization's API keys through the Platform API's
organization API key endpoints. Listing needs a key with api-read;
creating, rotating and deleting need api-admin. If your key can see more than
one organization, pass --org."#,
        r#"  mixlayer keys list
  mixlayer keys create --name ci --permissions inference
  mixlayer keys create --name claude-code --permissions inference,api-read --save
  mixlayer keys rotate <key-id> --yes"#,
    ),
    (
        r#"keys create"#,
        r#"Create an API key. The secret is printed once, alone on stdout, so you can
pipe it straight into a secret store; the note about it goes to stderr. With
--json the whole key object is printed, secret included. With --save the CLI
stores it in its credentials file (mode 0600) and prints no secret.

Permissions, as in the console: inference runs models; api-read reads the
management API; api-write also writes to it; api-admin also manages billing,
organizations and API keys. Default: inference."#,
        r#"  mixlayer keys create --name ci --permissions inference
  mixlayer keys create --name agent --permissions inference,api-read --expires-at 2027-01-01T00:00:00Z
  mixlayer keys create --name laptop --save
  mixlayer keys create --name ci --dry-run"#,
    ),
    (
        r#"keys delete"#,
        r#"Delete an API key. Anything using it stops working; model calls can accept
it for up to a minute while the gateway's key cache refreshes. Asks first;
scripts and agents must pass --yes explicitly (--agent does not imply it here)."#,
        r#"  mixlayer keys delete <key-id> --yes"#,
    ),
    (
        r#"keys list"#,
        r#"List API keys with their names, permissions, active state and expiry.
Secrets are never returned."#,
        r#"  mixlayer keys list
  mixlayer keys list --agent --select id,name,permissions"#,
    ),
    (
        r#"keys rotate"#,
        r#"Give an API key a new secret, printed once. The old secret stops working;
model calls can accept it for up to a minute while the gateway's key cache
refreshes. Asks first; scripts and agents must pass --yes explicitly."#,
        r#"  mixlayer keys rotate <key-id> --yes
  mixlayer keys rotate <key-id> --yes --save"#,
    ),
];

pub fn lookup(path: &str) -> Option<(&'static str, &'static str)> {
    HELP.iter().find(|(p, _, _)| *p == path).map(|(_, l, e)| (*l, *e))
}
