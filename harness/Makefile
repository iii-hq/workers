include $(dir $(abspath $(lastword $(MAKEFILE_LIST))))/Makefile.common

FOREGROUND_SCRIPT := $(MAKEFILE_DIR)scripts/foreground-dev.sh

GOAL_ARGS        := $(filter-out help install-iii-next build install-local clean cargo-clean dev-up dev-down dev-restart status smoke where,$(MAKECMDGOALS))
SELECTED_WORKERS := $(strip $(GOAL_ARGS))
UNKNOWN_WORKERS  := $(filter-out $(STACK),$(SELECTED_WORKERS))
ACTIVE_WORKERS   := $(if $(SELECTED_WORKERS),$(SELECTED_WORKERS),$(STACK))

.DEFAULT_GOAL := help

.PHONY: help install-iii-next build install-local clean cargo-clean dev-up dev-down dev-restart \
        status smoke where $(STACK)

help:
	@printf '%s\n' \
	  'Harness foreground development targets:' \
	  '  make dev-up                         start the stack in this terminal' \
	  '  make dev-down                       stop the foreground stack' \
	  '  make dev-restart                    restart the foreground stack' \
	  '  make status                         list registered workers' \
	  '  make smoke                          run basic engine/harness/router probes' \
	  '  make where                          print source roots and runtime mode' \
	  '  make build [workers...]             cargo build stack or selected workers' \
	  '  make cargo-clean [workers...]       cargo clean stack or selected workers' \
	  '  make install-local [workers...]     build and symlink into ~/.iii/workers' \
	  '' \
	  'Optional tmux mode: make tmux dev-up (from the repository root)'

install-iii-next:
	@curl -fsSL https://install.iii.dev/iii/main/install.sh | sh -s -- --next

build:
	@if [ -n "$(UNKNOWN_WORKERS)" ]; then \
	  echo "unknown worker(s): $(UNKNOWN_WORKERS)"; \
	  echo "valid workers: $(STACK)"; \
	  exit 2; \
	fi
	@for w in $(ACTIVE_WORKERS); do \
	  manifest="$(WORKTREE_ROOT_ABS)/$$w/Cargo.toml"; \
	  test -f "$$manifest" || { echo "missing $$manifest"; exit 2; }; \
	  echo "building $$w ($(PROFILE)) from $(WORKTREE_ROOT_ABS)"; \
	  cargo build $(BUILD_FLAG) --manifest-path "$$manifest" || exit 1; \
	done

install-local: build
	@mkdir -p "$(III_WORKERS)"
	@for w in $(ACTIVE_WORKERS); do \
	  ln -sfn "$(WORKTREE_ROOT_ABS)/$$w/target/$(PROFILE)/$$w" "$(III_WORKERS)/$$w"; \
	  echo "linked $(III_WORKERS)/$$w"; \
	done
	@echo "use make dev-up for foreground development, or make tmux dev-up for tmux development"

clean:
	@for w in $(STACK); do rm -f "$(III_WORKERS)/$$w"; done

cargo-clean:
	@if [ -n "$(UNKNOWN_WORKERS)" ]; then \
	  echo "unknown worker(s): $(UNKNOWN_WORKERS)"; \
	  echo "valid workers: $(STACK)"; \
	  exit 2; \
	fi
	@for w in $(ACTIVE_WORKERS); do \
	  manifest="$(WORKTREE_ROOT_ABS)/$$w/Cargo.toml"; \
	  test -f "$$manifest" || { echo "missing $$manifest"; exit 2; }; \
	  echo "cleaning cargo artifacts for $$w from $(WORKTREE_ROOT_ABS)"; \
	  cargo clean --manifest-path "$$manifest" || exit 1; \
	done

dev-up:
	@REPO_ROOT='$(REPO_ROOT)' \
	 WORKTREE_ROOT_ABS='$(WORKTREE_ROOT_ABS)' \
	 ENGINE_CONFIG='$(ENGINE_CONFIG)' \
	 ENGINE_URL='$(ENGINE_URL)' \
	 ENGINE_PORT='$(ENGINE_PORT)' \
	 CONSOLE_PORT='$(CONSOLE_PORT)' \
	 RUST_LOG='$(RUST_LOG)' \
	 RUN_PROFILE='$(RUN_PROFILE)' \
	 STACK='$(STACK)' \
	 PID_FILE='$(PID_FILE)' \
	 sh '$(FOREGROUND_SCRIPT)' up

dev-down:
	@REPO_ROOT='$(REPO_ROOT)' \
	 WORKTREE_ROOT_ABS='$(WORKTREE_ROOT_ABS)' \
	 ENGINE_CONFIG='$(ENGINE_CONFIG)' \
	 ENGINE_URL='$(ENGINE_URL)' \
	 ENGINE_PORT='$(ENGINE_PORT)' \
	 CONSOLE_PORT='$(CONSOLE_PORT)' \
	 RUST_LOG='$(RUST_LOG)' \
	 RUN_PROFILE='$(RUN_PROFILE)' \
	 STACK='$(STACK)' \
	 PID_FILE='$(PID_FILE)' \
	 sh '$(FOREGROUND_SCRIPT)' stop

dev-restart: dev-down dev-up

status:
	@iii trigger engine::workers::list --port "$(ENGINE_PORT)" --json '{}'

smoke:
	@set -e; \
	echo '== workers =='; \
	iii trigger engine::workers::list --port "$(ENGINE_PORT)" --json '{}'; \
	echo '== harness functions =='; \
	iii trigger engine::functions::list --port "$(ENGINE_PORT)" --json '{"prefix":"harness::"}'; \
	echo '== harness status =='; \
	iii trigger harness::status --port "$(ENGINE_PORT)" --json '{"session_id":"dev-smoke"}'; \
	echo '== sessions =='; \
	iii trigger session::list --port "$(ENGINE_PORT)" --json '{}'; \
	echo '== providers =='; \
	iii trigger router::provider::list --port "$(ENGINE_PORT)" --json '{}'

where:
	@echo "repo root:     $(REPO_ROOT)"
	@echo "worker root:   $(WORKTREE_ROOT_ABS)"
	@echo "engine config: $(ENGINE_CONFIG)"
	@echo "engine url:    $(ENGINE_URL)"
	@echo "runtime mode:  foreground"
	@echo "pid file:      $(PID_FILE)"
	@branch=$$(git -C "$(REPO_ROOT)" branch --show-current 2>/dev/null || true); \
	  sha=$$(git -C "$(REPO_ROOT)" rev-parse --short HEAD 2>/dev/null || true); \
	  if [ -n "$$sha" ]; then echo "repo ref:      $${branch:-detached} @ $$sha"; else echo "repo ref:      unavailable"; fi
	@branch=$$(git -C "$(WORKTREE_ROOT_ABS)" branch --show-current 2>/dev/null || true); \
	  sha=$$(git -C "$(WORKTREE_ROOT_ABS)" rev-parse --short HEAD 2>/dev/null || true); \
	  if [ -n "$$sha" ]; then echo "worker ref:    $${branch:-detached} @ $$sha"; else echo "worker ref:    unavailable"; fi

$(STACK):
	@:
