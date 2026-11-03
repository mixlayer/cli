# Feedback: a self-improving `mxl`

`mxl` can turn friction into GitHub issues on the public repo [mixlayer/cli](https://github.com/mixlayer/cli/issues) so the CLI improves where people actually get stuck. You always see and submit the issue yourself.

## Triggers

| Trigger | What's in the report |
| --- | --- |
| `mxl feedback [message]` | your note (scrubbed), `--kind`, `--command`, version, OS/arch |
| A command exits non-zero in a terminal | command name, exit code and its kind, version, OS/arch |
| The same failure twice within 10 minutes | as above, kind `friction`, with the repeat count |

Exit 6 (partial result, or PII found by `shield`) and 9 (decision gate matched) are expected outcomes and never trigger an offer.

## Consent

`feedback.mode` in `config.toml`, overridden by `MXL_FEEDBACK`:

```toml
[feedback]
mode = "ask"   # ask | always | never
```

- `ask` (default): after a failure, one `[y/N]` prompt on stderr. Answering yes opens the link in your browser and prints it.
- `always`: prints the link on stderr without asking. You still submit it.
- `never`: no offers, and `mxl feedback` exits with code 10.

There is never a prompt without a terminal on stdin and stderr, with `--no-input`, or in agent mode (`--agent` or a detected coding agent). The offer prints only to stderr and never changes the exit code.

## What is never captured

The report is built from an allowlist. Prompts, model output, error messages, API keys, organization or account IDs, environment values and file contents are never read into it. Only the command words are kept (`mxl ask "..."` becomes `ask`).

Your own note is scrubbed before it is shown: API keys and tokens (`sk-…`, `Bearer …`, `NAME_KEY=…`, long hex or base64 runs) become `[secret]`; paths (including your home directory) become `[path]`; emails, IPs, hostnames and URLs other than github.com and mixlayer.com become placeholders. Notes are capped at 500 characters.

`mxl feedback --dry-run` prints exactly the title, labels and body that would be filed, and the link, without contacting anything.

## Dedupe and rate limits

Every report has a fingerprint, `mxl-` plus 12 hex characters of SHA-256 over command, error kind and version. It's in the issue body, and before showing a link `mxl` searches GitHub's public, unauthenticated issue search for it. If an issue matches, you get its link to add a 👍 or comment instead of a duplicate. If the search fails (offline, rate limited), you get the new-issue link.

Locally, `feedback.json` in the state directory (mode 0600) allows at most one offer per fingerprint per 24 hours and five per day. It stores only fingerprints and timestamps.

## Title and labels

Every title starts with `[mxl-feedback]`. Labels: `feedback` and `automated` on every report, then the source (`auto` after a failure, `user-report` from `mxl feedback`), `kind:<kind>`, `cmd:<command>`, `exit:<code>`, `v<version>`, `os:<os>-<arch>`. GitHub only applies labels that exist in the repo, and only for people who can triage, so maintainers may need to create them.

## Future: a relay

v1 needs a GitHub account and a click. A later version could POST the same scrubbed report to a small Mixlayer-run relay (for example a Cloudflare Worker holding a GitHub App token scoped to this repo's issues) that re-scrubs, dedupes, rate-limits and files or +1s the issue. The CLI would still never hold a GitHub token. Not built.

Pattern credit: Yap Studio's feedback loop (`docs/feedback.md` in cathrynlavery/yap-studio).
