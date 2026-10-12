// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

//! Long help and examples for commands added after the Go port. Maintained by
//! hand: `help_text.rs` is regenerated from the Go snapshots, so entries there
//! would be overwritten. Same shape: (path, long, example).

pub const HELP: &[(&str, &str, &str)] = &[
    (
        r#"agent setup"#,
        r#"Install the Mixlayer Agent Skill and the local mxl-mcp server into
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
        r#"  mxl agent setup --agent claude-code
  mxl agent setup --agent all --yes
  mxl agent setup --agent claude-code --platform --yes
  mxl agent setup --agent codex --platform --key-env MIXLAYER_PLATFORM_KEY --yes
  mxl agent setup --agent all --uninstall --yes"#,
    ),
    (
        r#"keys"#,
        r#"Manage your organization's API keys through the Platform API's
organization API key endpoints. Listing needs a key with api-read;
creating, rotating and deleting need api-admin. If your key can see more than
one organization, pass --org."#,
        r#"  mxl keys list
  mxl keys create --name ci --permissions inference
  mxl keys create --name claude-code --permissions inference,api-read --save
  mxl keys rotate <key-id> --yes"#,
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
        r#"  mxl keys create --name ci --permissions inference
  mxl keys create --name agent --permissions inference,api-read --expires-at 2027-01-01T00:00:00Z
  mxl keys create --name laptop --save
  mxl keys create --name ci --dry-run"#,
    ),
    (
        r#"keys delete"#,
        r#"Delete an API key. Anything using it stops working; model calls can accept
it for up to a minute while the gateway's key cache refreshes. Asks first;
scripts and agents must pass --yes explicitly (--agent does not imply it here)."#,
        r#"  mxl keys delete <key-id> --yes"#,
    ),
    (
        r#"keys list"#,
        r#"List API keys with their names, permissions, active state and expiry.
Secrets are never returned."#,
        r#"  mxl keys list
  mxl keys list --agent --select id,name,permissions"#,
    ),
    (
        r#"keys rotate"#,
        r#"Give an API key a new secret, printed once. The old secret stops working;
model calls can accept it for up to a minute while the gateway's key cache
refreshes. Asks first; scripts and agents must pass --yes explicitly."#,
        r#"  mxl keys rotate <key-id> --yes
  mxl keys rotate <key-id> --yes --save"#,
    ),
    (
        r#"keys mint"#,
        r#"Create a short-lived API key, run one command with its secret in an
environment variable, then delete the key. The server also expires it after
--ttl even if this process stops. The child inherits stdin, stdout and stderr.
Model calls may keep accepting a deleted key for up to a minute while the
gateway's authorization cache refreshes."#,
        r#"  mxl keys mint --ttl 30m -- sh -c 'mxl models list'
  mxl keys mint --permissions inference --key-env JOB_KEY -- ./job"#,
    ),
    (
        r#"whoami"#,
        r#"Show which configured API key is in use, where it came from, and
which Platform API permissions it has. The secret is never shown. An
inference-only key can still show its local ID and source, but cannot read
its key record from the Platform API."#,
        r#"  mxl whoami
  mxl whoami --org <org-id> --agent"#,
    ),
    (
        r#"spend"#,
        r#"Show usage spend for the current billing period or a recent range.
--fail-over compares gross usage before credits in the chosen range and
exits 9 when usage is greater than the given dollar amount."#,
        r#"  mxl spend
  mxl spend --range 30d --fail-over 5 --agent"#,
    ),
    (
        r#"budget"#,
        r#"Show monthly spending controls and prepaid balance. A monthly
limit pauses paid model traffic when reached. Use budget set to change a
control; only the fields you pass are changed."#,
        r#"  mxl budget
  mxl budget show --org <org-id>
  mxl budget set --monthly-limit 25 --warn-at 20 --yes"#,
    ),
    (
        r#"budget show"#,
        r#"Show the monthly usage limit, warning, low-balance warning,
cutoff state and, for prepaid accounts, credit balance."#,
        r#"  mxl budget show --agent"#,
    ),
    (
        r#"budget set"#,
        r#"Change selected spend controls while preserving the others.
Values are positive dollars with at most two decimals; off disables a
control. The monthly limit pauses paid model traffic when reached. Pass
--yes explicitly, including in agent mode."#,
        r#"  mxl budget set --monthly-limit 25 --warn-at 20 --yes
  mxl budget set --monthly-limit off --yes
  mxl budget set --warn-at 15 --dry-run"#,
    ),
    (
        r#"audit"#,
        r#"List organization audit events newest first. Filter by time,
action prefix, actor or resource type. --follow prints the initial page
oldest first and polls for newer entries; with --json or --agent it emits
one JSON object per line until Ctrl-C."#,
        r#"  mxl audit --since 24h --action api_key.
  mxl audit --follow --interval 5s"#,
    ),
];

pub fn lookup(path: &str) -> Option<(&'static str, &'static str)> {
    HELP.iter().find(|(p, _, _)| *p == path).map(|(_, l, e)| (*l, *e))
}
