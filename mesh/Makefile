GO ?= go
BIN ?= bin/mesh
VERSION ?= $(shell sed -n 's/.*"version": "\([^"]*\)".*/\1/p' package.json)
LDFLAGS ?= -s -w
# Drops tailscale's peer API server, a host listener outside the tunnel; the guard in main_test.go fails without it.
TAGS ?= ts_omit_peerapiserver

.PHONY: build test ui

build:
	mkdir -p $(dir $(BIN))
	CGO_ENABLED=0 GOWORK=off $(GO) build -mod=readonly -trimpath -tags="$(TAGS)" -ldflags="$(LDFLAGS) -X main.version=$(VERSION)" -o $(BIN) .

ui:
	pnpm --dir ui build

test:
	$(GO) test -tags="$(TAGS)" ./...
