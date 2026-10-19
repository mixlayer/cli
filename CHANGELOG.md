# Changelog

## v0.3.1 (2026-10-19)

- `mxl shield scan`, `redact` and `restructure` read piped stdin when no file is given, so `echo "..." | mxl shield scan` works like `mxl shield scan -`. Before, a pipe printed the help text.
- `mxl models list --live-prices` shows public list prices without an API key. It reads the same public price feed as mixlayer.com/pricing and `mxl models sync`, so models (including DeepSeek) no longer show "unknown". The built-in list is still the fallback when the feed can't be reached. It also fills prices when your key can list models but not your organization's prices.

## v0.3.0 (2026-10-13)

- The command is now `mxl`; the MCP executable is `mxl-mcp`. Re-running the installer points an existing `mixlayer` or `mixlayer-mcp` at the new names, so agent configs from v0.2 keep working.
- `mxl models sync` registers the current model list with Codex, OpenCode and Hermes; `mxl agent setup --models` includes provider registration. For Codex it also writes a Mixlayer model catalog used only by the `mixlayer` profile, so `/model` lists Mixlayer models with their real context windows. `mxl auth token` supports Codex's command-backed key lookup. `mxl doctor` reports harness registration.
- In a terminal, `mxl models sync` prints a checklist like `agent setup`: each agent, how many models it has, the config path under `~`, and how to start it on Mixlayer. Pipes, `--json` and agents still get the JSON report, which now includes a `models` count per agent.
- `mxl shield ask` prints its regex-detection warning as two short lines in a terminal. JSON output is unchanged.

## v0.2.1 (2026-10-12)

- The Agent Skill now tells an agent how to install the CLI when it's missing (installer, API key, `doctor`), so an agent given only the skill (Hermes, OpenClaw, `npx skills add`) can finish setup itself. The README lists OpenClaw alongside the other agents.
- `mixlayer agent setup` prints a readable checklist in a terminal (each agent, then ✓ installed / → will be installed / ! needs a step, with paths under `~`) instead of raw JSON, and says whether anything was written. Pipes, `--json`, `--agent` and detected agents still get the same JSON report as before.

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
