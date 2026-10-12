# Changelog

## Unreleased

- The Agent Skill now tells an agent how to install the CLI when it's missing (installer, API key, `doctor`), so an agent given only the skill (Hermes, OpenClaw, `npx skills add`) can finish setup itself. The README lists OpenClaw alongside the other agents.

## v0.2.0 (2026-10-12)

- After a CLI upgrade, `mixlayer agent setup --yes` now updates the Agent Skill it installed earlier, instead of reporting `modified_skip` and leaving the old one. Copies you edited are still left alone (setup records a hash of what it wrote, so it can tell them apart), and uninstall removes an older unedited copy too. Skills installed by v0.1.0 are recognized. `mixlayer doctor` reports each installed skill as `current`, `outdated`, `modified` or `deleted`, and says how to update an outdated one.
- `mixlayer agent setup --platform` registers Mixlayer's hosted Platform MCP server (`https://api.mixlayer.com/v1/mcp`) with Claude Code, Codex and Cursor, so an agent can manage organizations, members, API keys, analytics and billing. The agent config holds only the name of the environment variable with the key (`--key-env`, default `MIXLAYER_API_KEY`). If that variable is set, setup checks the key with one read-only request and reports whether the Platform API accepted it. Uninstall removes these entries too. See [docs/platform-mcp.md](docs/platform-mcp.md).
- New `mixlayer keys list|create|rotate|delete` manage API keys through the Platform API. `create` and `rotate` print the new secret once, alone on stdout (or in the object with `--json`), or store it with `--save`. `rotate` and `delete` require an explicit `--yes` (agent mode doesn't imply it).
- `mixlayer keys mint` runs one command with a short-lived key and deletes it when the command ends.
- `mixlayer whoami` shows the configured key's ID, source, organization and permissions without showing its secret.
- `mixlayer spend` shows current or recent usage spend and can exit 9 when gross usage exceeds `--fail-over`.
- `mixlayer budget` shows spend controls and prepaid balance; `budget set` changes selected controls with explicit `--yes`.
- `mixlayer audit` lists organization audit events or follows new events as they arrive.
- `--csv`, `--plain` and `--quiet` now do what their help says on every command that prints data, including `sql`, `grep`, `savings`, `ladder`, `vault list`, `shield audit` and `profile list`: real CSV (header row, RFC 4180 quoting, nested values as JSON), tab-separated text, or one value per line. `chat --quiet` and `ask --quiet` print just the answer. An explicit format flag now beats the JSON implied by agent mode; an explicit `--json` still wins.

## v0.1.0 (2026-10-09)

First public release of the Mixlayer CLI.

- Written in Rust: `mixlayer` and `mixlayer-mcp` are small self-contained binaries for macOS, Linux (statically linked) and Windows, on x86-64 and ARM64, with a one-line installer that checks release checksums.

- `chat`, `ask`, `models` for Mixlayer's OpenAI-compatible API, with a local SQLite ledger of prompts, answers, tokens and cost.
- `ladder`, `escalate`, `council`, `compare` and `savings` to compare models and price answers against a frontier baseline using live prices.
- `shield` and `vault` to mask personal data locally before any model call (Mixlayer, OpenAI, Anthropic, OpenRouter or a custom endpoint) and restore it in the answer.
- Agent support: `--agent` JSON mode, stable exit codes, `agent-context`, `which`, an embedded Agent Skill, the `mixlayer-mcp` server and `agent setup` for Claude Code, Codex, Cursor, Hermes and Claude Desktop.
