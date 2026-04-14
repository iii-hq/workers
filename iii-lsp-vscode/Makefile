.PHONY: image install test package-check build install-cursor clean

IMAGE ?= iii-lsp-vscode-dev
VSIX ?= iii-lsp.vsix
NPM_CACHE ?= /tmp/npm-cache

DOCKER_RUN = docker run --rm -u "$$(id -u):$$(id -g)" -e npm_config_cache=$(NPM_CACHE) -v "$(CURDIR):/workspace" -w /workspace $(IMAGE)

image:
	docker build -t $(IMAGE) .

install: image
	$(DOCKER_RUN) npm ci

test: image
	$(DOCKER_RUN) npm test

package-check: image
	$(DOCKER_RUN) npm run package:check

build: image
	$(DOCKER_RUN) npm ci
	$(DOCKER_RUN) npx vsce package --out $(VSIX)

install-cursor: build
	cursor --install-extension $(VSIX) --force

clean:
	rm -f $(VSIX) iii-lsp-cursor.vsix iii-lsp-smoke.vsix tmp.vsix
