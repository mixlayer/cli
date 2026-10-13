# parity

The Mixlayer CLI was first written in Go and is now Rust. This directory keeps
the Rust build honest against that original behavior.

- `go-agent-context.json`, `go-agent-context-jini.json`: the Go CLI's full
  command, flag and exit-code description. `gen_spec.py` turns it into
  `src/spec.rs`, and `tests/parity.rs` checks that `mxl agent-context`
  still matches it.
- `go-help.json`, `go-help-jini.json`: long help and examples per command.
  `gen_help.py` turns them into `src/help_text.rs`.
- `fixtures/`: a ledger database and schema written by the Go CLI (the Rust
  store must read and extend them), the Go MCP server's tool list, and the Go
  shield evaluation results the Rust detectors must match.
- `e2e/`: a black-box suite that runs the release binary against a local fake
  API, plus live tests against the real API (`-tags live`, free models only).

```sh
cargo build --release
cd parity/e2e && go test .
```
