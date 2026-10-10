# Intentional differences from Go

`../go-output/expected/` records the Go CLI's output. These cases replace
the Go output where Rust deliberately differs: Go's `--csv` printed raw
JSON for API responses, `--plain` printed indented JSON, `--quiet` printed
nothing, and `profile list` ignored all three. Rust prints real CSV,
tab-separated text, and one value per line, as the flag help promises.
