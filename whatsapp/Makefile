GO ?= go
PNPM ?= pnpm
BIN ?= bin/whatsapp
VERSION ?= $(shell sed -n 's/.*"version": "\([^"]*\)".*/\1/p' package.json)
LDFLAGS ?= -s -w

.PHONY: build migrate test ui catalog generate

build:
	mkdir -p $(dir $(BIN))
	CGO_ENABLED=0 GOWORK=off $(GO) build -mod=readonly -trimpath -ldflags="$(LDFLAGS) -X main.version=$(VERSION)" -o $(BIN) .

migrate:
	mkdir -p bin
	CGO_ENABLED=0 $(GO) build -trimpath -ldflags="$(LDFLAGS)" -o bin/whatsapp-migrate ./cmd/whatsapp-migrate

test:
	$(GO) test ./...

ui:
	$(PNPM) --dir ui build
	mkdir -p assets
	cp ui/dist/page.js ui/dist/styles.css assets/

catalog:
	$(GO) run . --catalog

generate:
	$(GO) run ./cmd/cataloggen
