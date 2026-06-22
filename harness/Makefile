III_WORKERS ?= $(HOME)/.iii/workers
PROFILE     ?= release
# ROOT = the workers/ parent dir (every worker is a sibling of harness/)
ROOT        := $(abspath ..)
WORKERS     := harness iii-directory session-manager context-manager \
               provider-anthropic provider-openai shell web
# add llm-router above to also build/link it (sibling, not a declared dep)
CARGO_FLAG  := $(if $(filter release,$(PROFILE)),--release,)

.DEFAULT_GOAL := harness
.PHONY: harness clean

harness:
	@for w in $(WORKERS); do \
	  echo "▶ building $$w ($(PROFILE))"; \
	  cargo build $(CARGO_FLAG) --manifest-path $(ROOT)/$$w/Cargo.toml || exit 1; \
	  ln -sfn "$(ROOT)/$$w/target/$(PROFILE)/$$w" "$(III_WORKERS)/$$w"; \
	  echo "✔ linked $(III_WORKERS)/$$w"; \
	done
	@echo "↻ restart the engine (iii) to load the new binaries"

clean:                                # remove symlinks; reinstall with `iii worker add`
	@for w in $(WORKERS); do rm -f "$(III_WORKERS)/$$w"; done
