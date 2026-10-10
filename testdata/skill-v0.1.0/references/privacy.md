# Privacy round trip

Use fictional data in these examples. Replace paths and choose a detector based on where raw text may go.

```sh
mixlayer shield scan sample.txt --detector regex --agent
mixlayer shield redact sample.txt --detector regex --output masked.txt --agent
mixlayer shield ask "Summarize the file" --data sample.txt --upstream mixlayer --detector regex --dry-run --agent
mixlayer shield ask "Summarize the file" --data sample.txt --upstream mixlayer --detector local:example --agent
mixlayer shield audit --agent
```

The dry run previews masked outbound messages without a network call. `local:<ollama model>` detects on loopback. `mixlayer:<model>` sends unmasked input once to Mixlayer for detection; the answer model receives the masked payload. An external upstream such as OpenAI or Anthropic should use a model detector, with an explicit `--upstream` and `--model`. Regex-only external use requires `--accept-regex-only`; it is not a guarantee. Model detectors are incompatible with `--dry-run` because their detection requires a call.

`shield ask` restores only known placeholders from this request. The receipt records `masked_counts`, `tripwire_detector`, `leaked_pii_count`, `restored_count`, `unrestored_count`, `unrestored_tokens`, detector cost, upstream host, and payload hash. Inspect `unrestored_tokens` and `leaked_pii_count` before using an answer. A zero count does not prove all private data was found. Never print vault values.

Measure a detector against labeled synthetic examples before relying on it:

```sh
mixlayer shield eval --sample testdata/shield-eval/synthetic.jsonl --detector regex --agent
```

The report includes recall, precision, leak rate, and per-kind counts. Evaluation uses an isolated temporary vault. The default recall threshold can make the command exit 2 after the report; examine the report rather than treating that as a transport error.
