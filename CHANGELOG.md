# Changelog

## v0.1.0 (unreleased)

First public release of the Mixlayer CLI.

- `chat`, `ask`, `models` for Mixlayer's OpenAI-compatible API, with a local SQLite ledger of prompts, answers, tokens and cost.
- `ladder`, `escalate`, `council`, `compare` and `savings` to compare models and price answers against a frontier baseline using live prices.
- `shield` and `vault` to mask personal data locally before any model call (Mixlayer, OpenAI, Anthropic, OpenRouter or a custom endpoint) and restore it in the answer.
- Agent support: `--agent` JSON mode, stable exit codes, `agent-context`, `which`, an embedded Agent Skill, the `mixlayer-mcp` server and `agent setup` for Claude Code, Codex, Cursor, Hermes and Claude Desktop.
