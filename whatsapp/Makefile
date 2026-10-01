GO ?= go
PNPM ?= pnpm
BIN ?= bin/whatsapp

.PHONY: build test ui catalog generate

build:
	mkdir -p $(dir $(BIN))
	CGO_ENABLED=0 $(GO) build -trimpath -o $(BIN) .

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
