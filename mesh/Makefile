GO ?= go
BIN ?= bin/mesh
VERSION ?= $(shell sed -n 's/.*"version": "\([^"]*\)".*/\1/p' package.json)
LDFLAGS ?= -s -w

.PHONY: build test ui

build:
	mkdir -p $(dir $(BIN))
	CGO_ENABLED=0 GOWORK=off $(GO) build -mod=readonly -trimpath -ldflags="$(LDFLAGS) -X main.version=$(VERSION)" -o $(BIN) .

ui:
	pnpm --dir ui build

test:
	$(GO) test ./...
