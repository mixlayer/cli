# Experimental: Jini typed decisions

These commands call Mixlayer's decision endpoint (`/v1/systemone`), which is not yet publicly documented. They are hidden from help, `which`, `agent-context`, MCP and the embedded skill unless you set:

```sh
export MIXLAYER_ENABLE_JINI=1
```

## Usage

- `mxl decide --model <classification-model> --state-text "Masked facts" --noul 'valid=Is this valid?' --agent --dry-run` previews the exact request body and URL. Select an available classification model explicitly, or set `classification_model` in config; there is no default.
- `mxl decide --state facts.json --questions questions.json --fail-if 'risk>=0.8' --model <classification-model> --agent` preserves object/array state and merges the questions file with repeatable `--noul id=instructions`. Duplicate IDs are errors. `--state -` or `--questions -` reads stdin, one at a time.
- `mxl decide batch --input rows.jsonl --questions questions.json --model <classification-model> --concurrency 4 --max-rows 10000 --agent --dry-run` previews the first request and selected row count. CSV is also supported; `--state-field` selects one field, otherwise the whole row is state. Results are JSONL in completion order with one-based row IDs; errors do not abort later rows. Summary goes to stderr; any row failure exits 6.
- Decision gates support `id>n`, `id>=n`, `id<n`, `id<=n` for noul/score and `id==choice`. Any match prints the answer and exits 9. Noul scores are conditional yes scores, not calibrated confidence. Choice/score answers include confidence.
- `mxl escalate "Question" --checker jini --checker-model <classification-model> --threshold 0.8 --agent --dry-run` previews independent grading of each rung. Checker errors stop the climb. The default checker remains heuristic; checker scores, model, and usage are saved in the ledger's raw JSON.
- MCP `decide` accepts inline `state_text` and a `questions` JSON object; no files are required. Use `dry_run` before sending data. This non-destructive operation still consumes inference credits.

Systemone limits: 8 MiB body, 4 MiB rendered state, 1–4096 questions and expanded branches, and 16 KiB per expanded question. HTTP 422 exits 2 without retry; 529 exits 5. Only 429/529 retry, at most three times with exponential backoff and `Retry-After`; a delay above 30 seconds returns the error immediately.

