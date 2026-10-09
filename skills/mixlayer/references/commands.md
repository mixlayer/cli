# Mixlayer commands

Generated from the Cobra command tree by `go generate ./skills/mixlayer`. Do not edit manually. Use `--help` for detailed syntax.

## `mixlayer agent setup`

Preview or install the skill and MCP server for agents

Flags: --agent, --compact, --config, --csv, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --uninstall, --yes

## `mixlayer agent skill path`

Extract the embedded skill and print its directory

Flags: --agent, --compact, --config, --csv, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mixlayer agent skill print`

Print the embedded SKILL.md

Flags: --agent, --compact, --config, --csv, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mixlayer agent-context`

Emit structured JSON describing this CLI for agents

Flags: --agent, --compact, --config, --csv, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --plain, --pretty, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mixlayer ask`

Ask Mixlayer and save the prompt, answer, and reasoning locally

Flags: --agent, --compact, --config, --csv, --db, --dry-run, --home, --human-friendly, --json, --model, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --seed, --select, --show-thinking, --timeout, --yes

## `mixlayer auth`

Manage authentication for Mixlayer

Flags: --agent, --compact, --config, --csv, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mixlayer auth logout`

Clear stored credentials

Flags: --agent, --compact, --config, --csv, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mixlayer auth set-token`

Save an API key to the credentials file

Flags: --agent, --compact, --config, --csv, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mixlayer auth setup`

Print steps for obtaining a credential (use --launch to open the URL)

Flags: --agent, --compact, --config, --csv, --dry-run, --home, --human-friendly, --json, --launch, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mixlayer auth status`

Show authentication status

Flags: --agent, --compact, --config, --csv, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mixlayer chat`

Create a chat completion for the given model and messages

Flags: --agent, --compact, --config, --csv, --dry-run, --frequency-penalty, --home, --human-friendly, --json, --max-completion-tokens, --max-tokens, --messages, --model, --no-cache, --no-color, --no-input, --plain, --presence-penalty, --profile, --quiet, --rate-limit, --reasoning-effort, --repetition-penalty, --response-format, --seed, --select, --stop, --stream, --temperature, --thinking, --timeout, --tool-choice, --tools, --top-k, --top-p, --yes

## `mixlayer compare`

Compare a cheap rung with the frontier and price the delta

Flags: --agent, --cheap, --compact, --config, --csv, --db, --dry-run, --frontier, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mixlayer council`

Fan out to several rungs and ask a judge model to synthesize

Flags: --agent, --compact, --config, --csv, --db, --dry-run, --home, --human-friendly, --json, --judge, --members, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mixlayer doctor`

Check CLI health

Flags: --agent, --compact, --config, --csv, --dry-run, --fail-on, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mixlayer escalate`

Climb the model ladder only until a rung is confident enough

Flags: --agent, --checker, --compact, --confidence, --config, --csv, --db, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --rungs, --select, --timeout, --yes

## `mixlayer grep`

Search saved prompts, answers, and reasoning

Flags: --agent, --compact, --config, --csv, --db, --dry-run, --home, --human-friendly, --json, --limit, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mixlayer ladder`

Run one prompt across selected Mixlayer model rungs

Flags: --agent, --compact, --config, --csv, --db, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --reasoning, --rungs, --seed, --select, --timeout, --yes

## `mixlayer models`

Browse the model catalog

Flags: --agent, --compact, --config, --csv, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mixlayer models get`

Retrieve a model by ID

Flags: --agent, --compact, --config, --csv, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mixlayer models list`

List all available models

Flags: --agent, --compact, --config, --csv, --db, --dry-run, --home, --human-friendly, --json, --live-prices, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mixlayer models query`

Query the model catalog cache with a small DSL

Flags: --agent, --compact, --config, --csv, --db, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --refresh, --select, --timeout, --yes

## `mixlayer profile`

Named sets of flags saved for reuse

Flags: --agent, --compact, --config, --csv, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mixlayer profile delete`

Remove a profile

Flags: --agent, --compact, --config, --csv, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mixlayer profile list`

List saved profiles

Flags: --agent, --compact, --config, --csv, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mixlayer profile save`

Save the current invocation's non-default flags as a named profile

Flags: --agent, --compact, --config, --csv, --description, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mixlayer profile show`

Show a profile's values as JSON

Flags: --agent, --compact, --config, --csv, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mixlayer profile use`

Print the flag values a profile will apply (does not execute anything)

Flags: --agent, --compact, --config, --csv, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mixlayer replay`

Re-run a saved prompt with its original model and seed

Flags: --agent, --compact, --config, --csv, --db, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mixlayer savings`

Roll up Mixlayer ledger cost against a frontier baseline

Flags: --agent, --compact, --config, --csv, --db, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --vs, --yes

## `mixlayer shield`

Privacy firewall for masking data before frontier-model calls

Flags: --agent, --compact, --config, --csv, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mixlayer shield ask`

Ask a frontier model using only masked local data

Flags: --accept-regex-only, --agent, --compact, --config, --csv, --data, --db, --detector, --detector-base-url, --detector-fallback, --detector-key-env, --detector-timeout, --dry-run, --force, --guard, --home, --human-friendly, --json, --mask-columns, --model, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --show-sent, --show-thinking, --timeout, --upstream, --upstream-base-url, --upstream-key-env, --yes

## `mixlayer shield audit`

Show privacy receipts

Flags: --agent, --compact, --config, --csv, --db, --dry-run, --home, --human-friendly, --json, --limit, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mixlayer shield dict`

Manage local terms that every shield detector masks

Flags: --agent, --compact, --config, --csv, --db, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mixlayer shield dict add`

Add or update a local dictionary term

Flags: --agent, --compact, --config, --csv, --db, --dry-run, --home, --human-friendly, --json, --kind, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mixlayer shield dict list`

List local dictionary terms

Flags: --agent, --compact, --config, --csv, --db, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mixlayer shield dict remove`

Remove a local dictionary term

Flags: --agent, --compact, --config, --csv, --db, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mixlayer shield eval`

Measure detector recall, precision, and masking leaks on JSONL samples

Flags: --agent, --compact, --config, --csv, --detector, --detector-base-url, --detector-fallback, --detector-key-env, --detector-timeout, --dry-run, --home, --human-friendly, --json, --min-recall, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --sample, --select, --timeout, --yes

## `mixlayer shield ingest`

Split, redact, and reassemble a large corpus with one shared vault

Flags: --agent, --compact, --config, --csv, --db, --detector, --detector-base-url, --detector-fallback, --detector-key-env, --detector-timeout, --dry-run, --home, --human-friendly, --json, --manifest, --max-bytes, --no-cache, --no-color, --no-input, --output, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mixlayer shield redact`

Mask a file and populate the local vault

Flags: --agent, --compact, --config, --csv, --db, --detector, --detector-base-url, --detector-fallback, --detector-key-env, --detector-timeout, --diff, --dry-run, --home, --human-friendly, --json, --mask-columns, --no-cache, --no-color, --no-input, --output, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mixlayer shield restructure`

Coarsen values to reduce re-identification risk

Flags: --agent, --bucket, --coarsen-dates, --compact, --config, --csv, --drop, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --output, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mixlayer shield scan`

Detect PII locally without an upstream call

Flags: --agent, --compact, --config, --csv, --db, --detector, --detector-base-url, --detector-fallback, --detector-key-env, --detector-timeout, --dry-run, --home, --human-friendly, --json, --max-risk, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mixlayer sql`

Run read-only SQL against the local reasoning ledger

Flags: --agent, --compact, --config, --csv, --db, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mixlayer vault`

Manage the local reversible pseudonym vault

Flags: --agent, --compact, --config, --csv, --db, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mixlayer vault list`

List vault tokens

Flags: --agent, --compact, --config, --csv, --db, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mixlayer vault purge`

Delete every local vault entry

Flags: --agent, --compact, --config, --csv, --db, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mixlayer vault rehydrate`

Replace vault tokens with original values locally

Flags: --agent, --compact, --config, --csv, --db, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mixlayer vault rotate`

Report rotation support for the current vault

Flags: --agent, --compact, --config, --csv, --db, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mixlayer version`

Print version

Flags: --agent, --compact, --config, --csv, --dry-run, --home, --human-friendly, --json, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes

## `mixlayer which`

Find the command that implements a capability

Flags: --agent, --compact, --config, --csv, --dry-run, --home, --human-friendly, --json, --limit, --no-cache, --no-color, --no-input, --plain, --profile, --quiet, --rate-limit, --select, --timeout, --yes
