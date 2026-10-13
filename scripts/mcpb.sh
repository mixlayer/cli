#!/bin/sh
# Assemble the Claude Desktop MCP bundle (dist/mcpb) from a release build,
# and pack it with the `mcpb` tool when it is installed.
set -eu
cd "$(dirname "$0")/.."
cargo build --release --locked
version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
rm -rf dist/mcpb
mkdir -p dist/mcpb/bin dist/mcpb/skills/mixlayer/references
cp target/release/mxl target/release/mxl-mcp dist/mcpb/bin/
sed "s/\"version\": \"0.0.0\"/\"version\": \"$version\"/" manifest.json >dist/mcpb/manifest.json
cp skills/mixlayer/SKILL.md dist/mcpb/skills/mixlayer/SKILL.md
cp skills/mixlayer/references/privacy.md skills/mixlayer/references/commands.md dist/mcpb/skills/mixlayer/references/
if command -v mcpb >/dev/null 2>&1; then
	mcpb pack dist/mcpb dist/mixlayer.mcpb
else
	echo "Bundle assembled in dist/mcpb; run: mcpb pack dist/mcpb dist/mixlayer.mcpb"
fi
