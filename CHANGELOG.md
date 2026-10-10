# Changelog

## Unreleased

- `mixlayer agent setup --platform` registers Mixlayer's hosted Platform MCP server (`https://api.mixlayer.com/v1/mcp`) with Claude Code, Codex and Cursor, so an agent can manage organizations, members, API keys, analytics and billing. The agent config holds only the name of the environment variable with the key (`--key-env`, default `MIXLAYER_API_KEY`). If that variable is set, setup checks the key with one read-only request and reports whether the Platform API accepted it. Uninstall removes these entries too. See [docs/platform-mcp.md](docs/platform-mcp.md).
- New `mixlayer keys list|create|rotate|delete` manage API keys through the Platform API. `create` and `rotate` print the new secret once, alone on stdout (or in the object with `--json`), or store it with `--save`. `rotate` and `delete` require an explicit `--yes` (agent mode doesn't imply it).
- `--csv`, `--plain` and `--quiet` now do what their help says on every command that prints data, including `sql`, `grep`, `savings`, `ladder`, `vault list`, `shield audit` and `profile list`: real CSV (header row, RFC 4180 quoting, nested values as JSON), tab-separated text, or one value per line. `chat --quiet` and `ask --quiet` print just the answer. An explicit format flag now beats the JSON implied by agent mode; an explicit `--json` still wins.

## v0.1.0 (2026-10-09)

First public release of the Mixlayer CLI.

- Written in Rust: `mixlayer` and `mixlayer-mcp` are small self-contained binaries for macOS, Linux (statically linked) and Windows, on x86-64 and ARM64, with a one-line installer that checks release checksums.

- `chat`, `ask`, `models` for Mixlayer's OpenAI-compatible API, with a local SQLite ledger of prompts, answers, tokens and cost.
- `ladder`, `escalate`, `council`, `compare` and `savings` to compare models and price answers against a frontier baseline using live prices.
- `shield` and `vault` to mask personal data locally before any model call (Mixlayer, OpenAI, Anthropic, OpenRouter or a custom endpoint) and restore it in the answer.
- Agent support: `--agent` JSON mode, stable exit codes, `agent-context`, `which`, an embedded Agent Skill, the `mixlayer-mcp` server and `agent setup` for Claude Code, Codex, Cursor, Hermes and Claude Desktop.
