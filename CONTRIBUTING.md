# Contributing

Thanks for helping improve the Mixlayer CLI.

## Setup

You need Rust (the toolchain in `rust-toolchain.toml`; rustup installs it for you) and, for the end-to-end suite, Go 1.26.

```sh
git clone https://github.com/mixlayer/cli
cd cli
cargo build --release      # target/release/mixlayer and mixlayer-mcp
cargo test                 # unit and parity tests, no network
cd parity/e2e && go test .   # the release binary against a local fake API
```

`go test -tags live .` in `parity/e2e` calls the real API with `MIXLAYER_API_KEY` and uses free models only. Set `MIXLAYER_LIVE_PAID=1` to allow one short paid call.

## Pull requests

- Keep each PR to one change and include a test. Tests must not need a network connection or a real key; use a local fake server.
- Run `cargo fmt`, `cargo clippy --all-targets -- -D warnings` and `cargo test` before opening the PR.
- If you add or rename a command or flag, update these in the same PR:
  - `src/spec.rs`, then regenerate the agent skill's command reference with `cargo run -- agent skill-ref > skills/mixlayer/references/commands.md` (CI checks this).
  - `README.md`: every top-level command needs an example or a sentence there (a test checks this).
  - `skills/mixlayer/SKILL.md`, if agents need guidance on when to use the command.
  - `CHANGELOG.md`, under Unreleased.
  - The local MCP server picks the command up as a tool automatically; mark it `mcp:hidden` if it handles secrets or private data.

  Users get the new skill when they rerun `mixlayer agent setup --yes`.
- Never put an API key in a command argument, a test fixture or a log line.

## Reporting bugs

Open an issue with the command you ran, what you expected and `mixlayer doctor --json` output. For security problems, see [SECURITY.md](SECURITY.md).
