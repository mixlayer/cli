---
name: mixlayer
description: Use Mixlayer models and OpenAI-compatible calls; compare models, or escalate from cheap to strong models only when needed. Mask personal data before sending it to any model, including Claude or GPT, restore it locally, and measure redaction. Check live models, prices, authentication, and credits.
---

# Mixlayer agent rules

## If the CLI isn't installed yet

This skill drives the `mixlayer` command. If `mixlayer version` fails, set it up before anything else:

1. Tell the user you're about to install the Mixlayer CLI, then on macOS or Linux run `curl -fsSL https://github.com/mixlayer/cli/releases/latest/download/install.sh | sh`. It checks the release checksums and puts `mixlayer` and `mixlayer-mcp` in `~/.local/bin`; add that to `PATH` if the installer says so. On Windows, download the archive from https://github.com/mixlayer/cli/releases/latest and put both binaries on `PATH`.
2. The user needs an API key from https://console.mixlayer.com/app/api-keys (`mixlayer auth setup --launch` opens it). Ask them to export it as `MIXLAYER_API_KEY` or pipe it into `mixlayer auth set-token` themselves. Never ask them to paste the key into chat.
3. Run `mixlayer doctor --json` and fix anything it reports before the first call.
4. Optional, with the user's OK: `mixlayer agent setup --agent <claude-code|codex|cursor|hermes|claude-desktop> --yes` also adds the local MCP server to that agent.

- Authenticate through `MIXLAYER_API_KEY` in the environment. Never put a key on the command line or in an agent config.
- Use `--agent` for noninteractive JSON and `--select` to limit fields. Inspect `--dry-run` before any command that sends data or spends credits.
- Read available models and current prices with `mixlayer models list --live-prices --agent`; choose from that result. Never hard-code a model ID.
- Check setup with `mixlayer doctor --json`; use `mixlayer which "capability" --json` to find commands. If doctor lists an `agent_skills` entry as `outdated`, this skill is older than the installed CLI: ask the user to run `mixlayer agent setup --yes`, since it edits their agent config.
- Use `mixlayer ladder "question" --agent --dry-run` to compare answers, reasoning, latency, and cost across models. Use `mixlayer escalate "question" --agent --dry-run` when a cheap answer may be sufficient and a stronger model should be called only if needed.

## Organization and API keys

- Run `mixlayer whoami --agent` before organization work and check `can.admin` before administrative changes.
- For organization work (API keys, members, analytics, billing), use the `mixlayer-platform` MCP server when it is configured; otherwise start with `mixlayer keys list`. Reading needs `api-read`; creating, rotating or deleting keys needs `api-admin`.
- Give sub-agents and CI jobs a short-lived key with `mixlayer keys mint --ttl 30m -- mixlayer models list` instead of the user's key. Use `mixlayer spend --fail-over 5 --agent` as a guard in loops.
- Run `mixlayer budget set --monthly-limit 25 --yes` only when the user asked for that exact change. Use `mixlayer audit` to see what agents changed.
- Create keys with the fewest permissions the job needs (`--permissions inference` for a script that only calls models). Never echo a new secret into chat or a file the user did not ask for; prefer `mixlayer keys create --save` or piping stdout into a secret store.
- Rotating or deleting a key breaks whatever uses it. Do it only when the user asks for that key, and confirm the key ID first.

## Privacy before any model

Mixlayer chat completions have zero data retention by default: requests are processed in memory and discarded after inference, and are not used for training (https://docs.mixlayer.com/zero-data-retention). Do not tell users Mixlayer stores their prompts. Masking still keeps identities from the model itself and from providers with other retention rules. Mask personal data before sending it to Mixlayer, Claude, GPT, or any other model. Restore recognized placeholders locally after the response. See [privacy.md](references/privacy.md) for the full round trip and measurement.

| Detector | Decision |
| --- | --- |
| `regex` | Floor only; misses names, addresses, and international identifiers. Do not treat zero hits as proof of safety. |
| `local:<ollama model>` | Keeps detection and raw text on this machine; preferred for sensitive free text. |
| `mixlayer:<model>` | Sends raw text to Mixlayer once for detection before the masked model call. |

For a CSV whose private columns are known (customer, email, company, account), pass them with `--mask-columns customer,company` so every value in those columns is masked even when no detector recognizes it. Preview with `mixlayer shield redact <file> --mask-columns ... --agent` before `shield ask`.

For an external upstream, use a model detector. Regex-only requires the explicit CLI override and remains risky. A model detector cannot run in a no-network dry run; preview with regex, then run the selected detector. Inspect the receipt's `unrestored_tokens` and `leaked_pii_count`. Never claim zero leakage. Never print vault values.

## Exit codes and retry

| Code | Meaning | Agent action |
| --- | --- | --- |
| 0 | Success | Continue. |
| 1 | Other failure | Inspect error. |
| 2 | Usage error | Fix arguments; do not repeat unchanged. |
| 3 | Not found | Check ID or model. |
| 4 | Authentication or permission | Fix environment or access. |
| 5 | API failure | Inspect response before retrying. |
| 6 | Partial failure | Inspect individual results and receipt. |
| 7 | Rate limit | Honor `Retry-After` before retrying. |
| 8 | Billing or insufficient credits | Do not retry until resolved. |
| 9 | Decision gate matched (experimental) | Inspect answers before continuing. |
| 10 | Configuration failure | Fix configuration. |

The generated [command reference](references/commands.md) is a locator; use the live command help for detailed flags.
