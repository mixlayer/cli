.PHONY: build test test-e2e test-live lint install clean build-mcp install-mcp build-all mcpb

build:
	go build -o bin/mixlayer ./cmd/mixlayer

test:
	go test ./...

test-e2e:
	go test -count=1 ./e2e/

# Calls the real API with MIXLAYER_API_KEY (free models only unless MIXLAYER_LIVE_PAID=1).
test-live:
	go test -tags live -count=1 -v ./e2e/

lint:
	golangci-lint run

install:
	go install ./cmd/mixlayer

clean:
	rm -rf bin/

build-mcp:
	go build -o bin/mixlayer-mcp ./cmd/mixlayer-mcp

install-mcp:
	go install ./cmd/mixlayer-mcp

build-all: build build-mcp

# The MCP server shells out to its sibling CLI for the core workflows.
mcpb: build-all
	mkdir -p dist/mcpb/bin dist/mcpb/skills/mixlayer/references
	cp bin/mixlayer bin/mixlayer-mcp dist/mcpb/bin/
	cp manifest.json dist/mcpb/manifest.json
	cp skills/mixlayer/SKILL.md dist/mcpb/skills/mixlayer/SKILL.md
	cp skills/mixlayer/references/privacy.md skills/mixlayer/references/commands.md dist/mcpb/skills/mixlayer/references/
	@if command -v mcpb >/dev/null 2>&1; then \
		mcpb pack dist/mcpb dist/mixlayer.mcpb; \
	else \
		printf '%s\n' 'Bundle assembled in dist/mcpb; run: mcpb pack dist/mcpb dist/mixlayer.mcpb'; \
	fi
