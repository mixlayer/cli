# Mixlayer commands

Generated from the command table by `mxl agent skill-ref`. Do not edit manually. Use `--help` for detailed syntax.

## `mxl agent setup`

Preview or install the skill and MCP server for agents

Flags: --agent, --compact, --config, --csv, --dry-run, --home, --human-friendly, --json, --key-env, --models, --no-cache, --no-color, --no-input, --plain, --platform, --profile, --quiet, --rate-limit, --select, --timeout, --uninstall, --yes

## `mxl agent skill path`

Extract the embedded skill and print its directory

Flags: --agent, --compact, --config, --csv, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mxl agent skill print`

Print the embedded SKILL.md

Flags: --agent, --compact, --config, --csv, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mxl agent-context`

Emit structured JSON describing this CLI for agents

Flags: --agent, --compact, --config, --csv, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --plain, --pretty, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mxl ask`

Ask Mixlayer and save the prompt, answer, and reasoning locally

Flags: --agent, --compact, --config, --csv, --db, --dry-run, --home, --human-friendly, --json, --model, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --seed, --select, --show-thinking, --timeout, --yes

## `mxl audit`

List or follow organization audit events

Flags: --action, --actor, --agent, --compact, --config, --csv, --dry-run, --follow, --home, --human-friendly, --interval, --json, --limit, --no-cache, --no-color, --no-input, --org, --plain, --profile, --quiet, --rate-limit, --resource-type, --select, --since, --timeout, --yes

## `mxl auth`

Manage authentication for Mixlayer

Flags: --agent, --compact, --config, --csv, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mxl auth logout`

Clear stored credentials

Flags: --agent, --compact, --config, --csv, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mxl auth set-token`

Save an API key to the credentials file

Flags: --agent, --compact, --config, --csv, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mxl auth setup`

Print steps for obtaining a credential (use --launch to open the URL)

Flags: --agent, --compact, --config, --csv, --dry-run, --home, --human-friendly, --json, --launch, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mxl auth status`

Show authentication status

Flags: --agent, --compact, --config, --csv, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mxl auth token`

Print the resolved API key for command-backed authentication

Flags: --agent, --compact, --config, --csv, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --reveal, --select, --timeout, --yes

## `mxl budget`

Show or set monthly spending controls

Flags: --agent, --compact, --config, --csv, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --org, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mxl budget set`

Change monthly spend controls (requires --yes)

Flags: --agent, --compact, --config, --csv, --dry-run, --home, --human-friendly, --json, --low-balance-warning, --monthly-limit, --no-cache, --no-color, --no-input, --org, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --warn-at, --yes

## `mxl budget show`

Show spending controls and prepaid balance

Flags: --agent, --compact, --config, --csv, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --org, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mxl chat`

Create a chat completion for the given model and messages

Flags: --agent, --compact, --config, --csv, --dry-run, --frequency-penalty, --home, --human-friendly, --json, --max-completion-tokens, --max-tokens, --messages, --model, --no-cache, --no-color, --no-input, --plain, --presence-penalty, --profile, --quiet, --rate-limit, --reasoning-effort, --repetition-penalty, --response-format, --seed, --select, --stop, --stream, --temperature, --thinking, --timeout, --tool-choice, --tools, --top-k, --top-p, --yes

## `mxl compare`

Compare a cheap rung with the frontier and price the delta

Flags: --agent, --cheap, --compact, --config, --csv, --db, --dry-run, --frontier, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mxl council`

Fan out to several rungs and ask a judge model to synthesize

Flags: --agent, --compact, --config, --csv, --db, --dry-run, --home, --human-friendly, --json, --judge, --members, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mxl doctor`

Check CLI health

Flags: --agent, --compact, --config, --csv, --dry-run, --fail-on, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mxl escalate`

Climb the model ladder only until a rung is confident enough

Flags: --agent, --checker, --compact, --confidence, --config, --csv, --db, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --rungs, --select, --timeout, --yes

## `mxl grep`

Search saved prompts, answers, and reasoning

Flags: --agent, --compact, --config, --csv, --db, --dry-run, --home, --human-friendly, --json, --limit, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mxl keys`

Create, list, mint, rotate and delete API keys through the Platform API

Flags: --agent, --compact, --config, --csv, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mxl keys create`

Create an API key and print its secret once

Flags: --agent, --compact, --config, --csv, --dry-run, --expires-at, --home, --human-friendly, --json, --name, --no-cache, --no-color, --no-input, --org, --permissions, --plain, --profile, --quiet, --rate-limit, --save, --select, --timeout, --yes

## `mxl keys delete`

Delete an API key (asks first unless --yes)

Flags: --agent, --compact, --config, --csv, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --org, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mxl keys list`

List API keys in your organization (never shows secrets)

Flags: --agent, --compact, --config, --csv, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --org, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mxl keys mint`

Run a command with a short-lived API key, then delete it

Flags: --agent, --compact, --config, --csv, --dry-run, --home, --human-friendly, --json, --key-env, --name, --no-cache, --no-color, --no-input, --org, --permissions, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --ttl, --yes

## `mxl keys rotate`

Replace a key's secret; the old secret stops working (asks first unless --yes)

Flags: --agent, --compact, --config, --csv, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --org, --plain, --profile, --quiet, --rate-limit, --save, --select, --timeout, --yes

## `mxl ladder`

Run one prompt across selected Mixlayer model rungs

Flags: --agent, --compact, --config, --csv, --db, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --reasoning, --rungs, --seed, --select, --timeout, --yes

## `mxl models`

Browse the model catalog

Flags: --agent, --compact, --config, --csv, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mxl models get`

Retrieve a model by ID

Flags: --agent, --compact, --config, --csv, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mxl models list`

List all available models

Flags: --agent, --compact, --config, --csv, --db, --dry-run, --home, --human-friendly, --json, --live-prices, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mxl models query`

Query the model catalog cache with a small DSL

Flags: --agent, --compact, --config, --csv, --db, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --refresh, --select, --timeout, --yes

## `mxl models sync`

Register the current Mixlayer models in agent harnesses

Flags: --agent, --compact, --config, --csv, --default-model, --dry-run, --force, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --uninstall, --yes

## `mxl profile`

Named sets of flags saved for reuse

Flags: --agent, --compact, --config, --csv, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mxl profile delete`

Remove a profile

Flags: --agent, --compact, --config, --csv, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mxl profile list`

List saved profiles

Flags: --agent, --compact, --config, --csv, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mxl profile save`

Save the current invocation's non-default flags as a named profile

Flags: --agent, --compact, --config, --csv, --description, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mxl profile show`

Show a profile's values as JSON

Flags: --agent, --compact, --config, --csv, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mxl profile use`

Print the flag values a profile will apply (does not execute anything)

Flags: --agent, --compact, --config, --csv, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mxl replay`

Re-run a saved prompt with its original model and seed

Flags: --agent, --compact, --config, --csv, --db, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mxl savings`

Roll up Mixlayer ledger cost against a frontier baseline

Flags: --agent, --compact, --config, --csv, --db, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --vs, --yes

## `mxl shield`

Privacy firewall for masking data before frontier-model calls

Flags: --agent, --compact, --config, --csv, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mxl shield ask`

Ask a frontier model using only masked local data

Flags: --accept-regex-only, --agent, --compact, --config, --csv, --data, --db, --detector, --detector-base-url, --detector-fallback, --detector-key-env, --detector-timeout, --dry-run, --force, --guard, --home, --human-friendly, --json, --mask-columns, --model, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --show-sent, --show-thinking, --timeout, --upstream, --upstream-base-url, --upstream-key-env, --yes

## `mxl shield audit`

Show privacy receipts

Flags: --agent, --compact, --config, --csv, --db, --dry-run, --home, --human-friendly, --json, --limit, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mxl shield dict`

Manage local terms that every shield detector masks

Flags: --agent, --compact, --config, --csv, --db, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mxl shield dict add`

Add or update a local dictionary term

Flags: --agent, --compact, --config, --csv, --db, --dry-run, --home, --human-friendly, --json, --kind, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mxl shield dict list`

List local dictionary terms

Flags: --agent, --compact, --config, --csv, --db, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mxl shield dict remove`

Remove a local dictionary term

Flags: --agent, --compact, --config, --csv, --db, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mxl shield eval`

Measure detector recall, precision, and masking leaks on JSONL samples

Flags: --agent, --compact, --config, --csv, --detector, --detector-base-url, --detector-fallback, --detector-key-env, --detector-timeout, --dry-run, --home, --human-friendly, --json, --min-recall, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --sample, --select, --timeout, --yes

## `mxl shield ingest`

Split, redact, and reassemble a large corpus with one shared vault

Flags: --agent, --compact, --config, --csv, --db, --detector, --detector-base-url, --detector-fallback, --detector-key-env, --detector-timeout, --dry-run, --home, --human-friendly, --json, --manifest, --max-bytes, --no-cache, --no-color, --no-input, --output, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mxl shield redact`

Mask a file and populate the local vault

Flags: --agent, --compact, --config, --csv, --db, --detector, --detector-base-url, --detector-fallback, --detector-key-env, --detector-timeout, --diff, --dry-run, --home, --human-friendly, --json, --mask-columns, --no-cache, --no-color, --no-input, --output, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mxl shield restructure`

Coarsen values to reduce re-identification risk

Flags: --agent, --bucket, --coarsen-dates, --compact, --config, --csv, --drop, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --output, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mxl shield scan`

Detect PII locally without an upstream call

Flags: --agent, --compact, --config, --csv, --db, --detector, --detector-base-url, --detector-fallback, --detector-key-env, --detector-timeout, --dry-run, --home, --human-friendly, --json, --max-risk, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mxl spend`

Show usage spend and optionally gate on a dollar amount

Flags: --agent, --compact, --config, --csv, --dry-run, --fail-over, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --org, --plain, --profile, --quiet, --range, --rate-limit, --select, --timeout, --yes

## `mxl sql`

Run read-only SQL against the local reasoning ledger

Flags: --agent, --compact, --config, --csv, --db, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mxl vault`

Manage the local reversible pseudonym vault

Flags: --agent, --compact, --config, --csv, --db, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mxl vault list`

List vault tokens

Flags: --agent, --compact, --config, --csv, --db, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mxl vault purge`

Delete every local vault entry

Flags: --agent, --compact, --config, --csv, --db, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mxl vault rehydrate`

Replace vault tokens with original values locally

Flags: --agent, --compact, --config, --csv, --db, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mxl vault rotate`

Report rotation support for the current vault

Flags: --agent, --compact, --config, --csv, --db, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mxl version`

Print version

Flags: --agent, --compact, --config, --csv, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mxl which`

Find the command that implements a capability

Flags: --agent, --compact, --config, --csv, --dry-run, --home, --human-friendly, --json, --limit, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mxl whoami`

Show this key's identity and Platform permissions

Flags: --agent, --compact, --config, --csv, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --org, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes
