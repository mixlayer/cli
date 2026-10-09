# Contributing

Thanks for helping improve the Mixlayer CLI.

## Setup

You need Go (the version in `go.mod`; `GOTOOLCHAIN=auto` fetches it for you).

```sh
git clone https://github.com/mixlayer/cli
cd cli
make build          # ./bin/mixlayer
make test           # unit tests, no network
make test-e2e       # runs the built binary against a local fake API
```

`make test-live` calls the real API with `MIXLAYER_API_KEY` and uses free models only. Set `MIXLAYER_LIVE_PAID=1` to allow one short paid call.

## Pull requests

- Keep each PR to one change and include a test. Tests must not need a network connection or a real key; use `httptest`.
- Run `gofmt`, `go vet ./...` and `make test` before opening the PR.
- If you add or rename a command or flag, run `go generate ./skills/mixlayer` so the agent skill's command reference stays current (CI checks this).
- Never put an API key in a command argument, a test fixture or a log line.

## Reporting bugs

Open an issue with the command you ran, what you expected and `mixlayer doctor --json` output. For security problems, see [SECURITY.md](SECURITY.md).
