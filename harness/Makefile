III_WORKERS  ?= $(HOME)/.iii/workers
PROFILE      ?= release
RUN_PROFILE  ?= debug
TMUX_SESSION ?= harness-dev
ENGINE_URL   ?= ws://127.0.0.1:49134
ENGINE_PORT  ?= 49134
CONSOLE_PORT ?= 3113
RUST_LOG     ?= info
PYTHON       ?= python3

MAKEFILE_DIR := $(dir $(abspath $(lastword $(MAKEFILE_LIST))))
REPO_ROOT    := $(abspath $(MAKEFILE_DIR)/..)
WORKTREE_ROOT ?= $(REPO_ROOT)
WORKTREE_ROOT_ABS := $(abspath $(if $(filter /%,$(WORKTREE_ROOT)),$(WORKTREE_ROOT),$(REPO_ROOT)/$(WORKTREE_ROOT)))
ENGINE_CONFIG ?= $(MAKEFILE_DIR)engine.config.yaml
SCRAPLING_ROOT := $(WORKTREE_ROOT_ABS)/scrapling
SCRAPLING_VENV := $(SCRAPLING_ROOT)/.venv
SCRAPLING_STAMP := $(SCRAPLING_VENV)/.installed

STACK := iii-directory llm-router session-manager context-manager \
         provider-anthropic provider-openai provider-openai-codex shell scrapling queue harness approval-gate console

GOAL_ARGS        := $(filter-out help install-iii-next build install-local clean cargo-clean dev-up dev-down dev-restart \
                    engine wait-engine restart restart-worker stop stop-worker stop-work stop-engine status smoke logs attach \
                    ensure-session prepare-scrapling require-engine where conformance-e2e,$(MAKECMDGOALS))
SELECTED_WORKERS := $(strip $(GOAL_ARGS))
ACTIVE_WORKERS   := $(if $(SELECTED_WORKERS),$(SELECTED_WORKERS),$(STACK))
UNKNOWN_WORKERS  := $(filter-out $(STACK),$(SELECTED_WORKERS))

BUILD_FLAG := $(if $(filter release,$(PROFILE)),--release,)
RUN_FLAG   := $(if $(filter release,$(RUN_PROFILE)),--release,)

.DEFAULT_GOAL := help

.PHONY: help install-iii-next build install-local clean cargo-clean dev-up dev-down dev-restart engine wait-engine \
        restart restart-worker stop stop-worker stop-work stop-engine status smoke logs attach ensure-session prepare-scrapling \
        require-engine where conformance-e2e $(STACK)

help:
	@printf '%s\n' \
	  'Harness local development targets:' \
	  '  make dev-up                         start engine + harness stack in tmux' \
	  '  make dev-down                       stop the tmux stack' \
	  '  make dev-restart                    restart all workers in the stack' \
	  '  make restart console harness shell  restart selected workers' \
	  '  make restart harness WORKTREE_ROOT=.worktrees/my-feature' \
	  '  make stop harness shell             stop selected worker windows' \
	  '  make stop-work                      stop all worker windows, keep engine' \
	  '  make stop-engine                    stop the engine window only' \
	  '  make install-iii-next               install the iii CLI @next channel' \
	  '  make status                         list registered workers' \
	  '  make smoke                          run basic engine/harness/router probes' \
	  '  make where                          print source roots, refs, and tmux panes' \
	  '  make logs                           attach to the tmux session' \
	  '  make build [workers...]             cargo build stack or selected workers' \
	  '  make cargo-clean [workers...]       cargo clean stack or selected workers' \
	  '  make install-local [workers...]     build and symlink into ~/.iii/workers' \
	  '  make conformance-e2e III_BIN=<iii>  run the harness conformance scenarios (evals/conformance)'

install-iii-next:
	@curl -fsSL https://install.iii.dev/iii/main/install.sh | sh -s -- --next

$(SCRAPLING_STAMP): $(SCRAPLING_ROOT)/pyproject.toml
	@test -x "$(SCRAPLING_VENV)/bin/python" || "$(PYTHON)" -m venv --without-pip "$(SCRAPLING_VENV)"
	@"$(PYTHON)" -m pip --python "$(SCRAPLING_VENV)/bin/python" install --disable-pip-version-check -e "$(SCRAPLING_ROOT)"
	@touch "$@"

prepare-scrapling: $(SCRAPLING_STAMP)

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
	@echo "restart the engine to load linked binaries, or use make dev-up for cargo-run development"

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

dev-up: engine wait-engine
	@$(MAKE) --no-print-directory restart $(STACK)
	@$(MAKE) --no-print-directory status
	@echo "console: http://127.0.0.1:$(CONSOLE_PORT)"
	@echo "tmux:    tmux attach -t $(TMUX_SESSION)"

dev-down:
	@tmux kill-session -t "$(TMUX_SESSION)" 2>/dev/null || true

dev-restart:
	@$(MAKE) --no-print-directory restart $(STACK)

ensure-session:
	@tmux has-session -t "$(TMUX_SESSION)" 2>/dev/null || \
	  tmux new-session -d -s "$(TMUX_SESSION)" -n _keepalive -c "$(REPO_ROOT)"

engine: ensure-session
	@if ! tmux list-windows -t "$(TMUX_SESSION)" -F '#W' | grep -Fxq engine; then \
	  listener=$$(lsof -tiTCP:$(ENGINE_PORT) -sTCP:LISTEN 2>/dev/null | head -n 1); \
	  if [ -n "$$listener" ]; then \
	    echo "cannot start engine in $(TMUX_SESSION):engine; TCP $(ENGINE_PORT) is already in use"; \
	    lsof -nP -iTCP:$(ENGINE_PORT) -sTCP:LISTEN || true; \
	    echo "stop the other engine first, then run make dev-up again"; \
	    exit 2; \
	  fi; \
	fi
	@cmd="cd '$(REPO_ROOT)' && iii -c '$(ENGINE_CONFIG)'"; \
	if tmux list-windows -t "$(TMUX_SESSION)" -F '#W' | grep -Fxq engine; then \
	  tmux respawn-window -k -t "$(TMUX_SESSION):engine" -c "$(REPO_ROOT)" "$$cmd"; \
	else \
	  tmux new-window -d -t "$(TMUX_SESSION)" -n engine -c "$(REPO_ROOT)" "$$cmd"; \
	fi; \
	echo "started engine in tmux: $(TMUX_SESSION):engine"

wait-engine:
	@i=0; \
	while [ $$i -lt 40 ]; do \
	  engine_cmd=$$(tmux display-message -p -t "$(TMUX_SESSION):engine" '#{pane_current_command}' 2>/dev/null || true); \
	  if [ "$$engine_cmd" = "iii" ] && \
	     iii trigger engine::workers::list --port "$(ENGINE_PORT)" --json '{}' >/dev/null 2>&1; then \
	    echo "engine ready in $(TMUX_SESSION):engine on $(ENGINE_URL)"; \
	    exit 0; \
	  fi; \
	  i=$$((i + 1)); \
	  sleep 0.5; \
	done; \
	echo "engine did not become ready in $(TMUX_SESSION):engine on $(ENGINE_URL)"; \
	exit 1

require-engine:
	@engine_cmd=$$(tmux display-message -p -t "$(TMUX_SESSION):engine" '#{pane_current_command}' 2>/dev/null || true); \
	if [ "$$engine_cmd" != "iii" ]; then \
	  echo "engine is not running in $(TMUX_SESSION):engine"; \
	  echo "run make dev-up or make engine first"; \
	  exit 2; \
	fi
	@iii trigger engine::workers::list --port "$(ENGINE_PORT)" --json '{}' >/dev/null || { \
	  echo "engine window exists, but $(ENGINE_URL) is not reachable"; \
	  exit 2; \
	}

restart:
	@if [ -z "$(SELECTED_WORKERS)" ]; then \
	  echo "usage: make restart <worker...>"; \
	  echo "valid workers: $(STACK)"; \
	  exit 2; \
	fi
	@if [ -n "$(UNKNOWN_WORKERS)" ]; then \
	  echo "unknown worker(s): $(UNKNOWN_WORKERS)"; \
	  echo "valid workers: $(STACK)"; \
	  exit 2; \
	fi
	@for w in $(SELECTED_WORKERS); do \
	  if [ "$$w" = "scrapling" ]; then \
	    manifest="$(SCRAPLING_ROOT)/pyproject.toml"; \
	  else \
	    manifest="$(WORKTREE_ROOT_ABS)/$$w/Cargo.toml"; \
	  fi; \
	  test -f "$$manifest" || { echo "missing $$manifest"; exit 2; }; \
	done
	@$(MAKE) --no-print-directory require-engine
	@$(MAKE) --no-print-directory ensure-session
	@for w in $(SELECTED_WORKERS); do \
	  $(MAKE) --no-print-directory restart-worker WORKER=$$w || exit 1; \
	done

restart-worker:
	@test -n "$(WORKER)" || { echo "WORKER is required"; exit 2; }
	@case " $(STACK) " in \
	  *" $(WORKER) "*) ;; \
	  *) echo "unknown worker: $(WORKER)"; echo "valid workers: $(STACK)"; exit 2 ;; \
	esac
	@worker="$(WORKER)"; \
	if [ "$$worker" = "scrapling" ]; then \
	  test -f "$(SCRAPLING_ROOT)/pyproject.toml" || { echo "missing $(SCRAPLING_ROOT)/pyproject.toml"; exit 2; }; \
	  $(MAKE) --no-print-directory prepare-scrapling || exit 1; \
	  cmd="cd '$(SCRAPLING_ROOT)' && III_URL='$(ENGINE_URL)' '$(SCRAPLING_VENV)/bin/python' -m src.main"; \
	else \
	  manifest="$(WORKTREE_ROOT_ABS)/$$worker/Cargo.toml"; \
	  test -f "$$manifest" || { echo "missing $$manifest"; exit 2; }; \
	  args="--url $(ENGINE_URL)"; \
	  if [ "$$worker" = "queue" ]; then args="$$args --config '$(MAKEFILE_DIR)queue.config.yaml'"; fi; \
	  if [ "$$worker" = "console" ]; then args="$$args --http-port $(CONSOLE_PORT)"; fi; \
	  cmd="cd '$(WORKTREE_ROOT_ABS)' && RUST_LOG='$(RUST_LOG)' cargo run $(RUN_FLAG) --manifest-path '$$manifest' -- $$args"; \
	fi; \
	if tmux list-windows -t "$(TMUX_SESSION)" -F '#W' | grep -Fxq "$$worker"; then \
	  tmux respawn-window -k -t "$(TMUX_SESSION):$$worker" -c "$(WORKTREE_ROOT_ABS)" "$$cmd"; \
	else \
	  tmux new-window -d -t "$(TMUX_SESSION)" -n "$$worker" -c "$(WORKTREE_ROOT_ABS)" "$$cmd"; \
	fi; \
	echo "restarted $$worker from $(WORKTREE_ROOT_ABS) in tmux: $(TMUX_SESSION):$$worker"

stop:
	@if [ -z "$(SELECTED_WORKERS)" ]; then \
	  echo "usage: make stop <worker...>"; \
	  echo "valid workers: $(STACK)"; \
	  exit 2; \
	fi
	@if [ -n "$(UNKNOWN_WORKERS)" ]; then \
	  echo "unknown worker(s): $(UNKNOWN_WORKERS)"; \
	  echo "valid workers: $(STACK)"; \
	  exit 2; \
	fi
	@for w in $(SELECTED_WORKERS); do \
	  $(MAKE) --no-print-directory stop-worker WORKER=$$w || exit 1; \
	done

stop-worker:
	@test -n "$(WORKER)" || { echo "WORKER is required"; exit 2; }
	@case " $(STACK) " in \
	  *" $(WORKER) "*) ;; \
	  *) echo "unknown worker: $(WORKER)"; echo "valid workers: $(STACK)"; exit 2 ;; \
	esac
	@if tmux list-windows -t "$(TMUX_SESSION)" -F '#W' 2>/dev/null | grep -Fxq "$(WORKER)"; then \
	  tmux kill-window -t "$(TMUX_SESSION):$(WORKER)"; \
	  echo "stopped $(WORKER) in tmux: $(TMUX_SESSION):$(WORKER)"; \
	else \
	  echo "$(WORKER) is not running in $(TMUX_SESSION)"; \
	fi

stop-work:
	@for w in $(STACK); do \
	  $(MAKE) --no-print-directory stop-worker WORKER=$$w || exit 1; \
	done

stop-engine:
	@if tmux list-windows -t "$(TMUX_SESSION)" -F '#W' 2>/dev/null | grep -Fxq engine; then \
	  tmux kill-window -t "$(TMUX_SESSION):engine"; \
	  echo "stopped engine in tmux: $(TMUX_SESSION):engine"; \
	else \
	  echo "engine is not running in $(TMUX_SESSION)"; \
	fi

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

logs attach:
	@tmux attach -t "$(TMUX_SESSION)"

where:
	@echo "repo root:     $(REPO_ROOT)"
	@echo "worker root:   $(WORKTREE_ROOT_ABS)"
	@echo "engine config: $(ENGINE_CONFIG)"
	@echo "engine url:    $(ENGINE_URL)"
	@echo "tmux session:  $(TMUX_SESSION)"
	@branch=$$(git -C "$(REPO_ROOT)" branch --show-current 2>/dev/null || true); \
	  sha=$$(git -C "$(REPO_ROOT)" rev-parse --short HEAD 2>/dev/null || true); \
	  if [ -n "$$sha" ]; then echo "repo ref:      $${branch:-detached} @ $$sha"; else echo "repo ref:      unavailable"; fi
	@branch=$$(git -C "$(WORKTREE_ROOT_ABS)" branch --show-current 2>/dev/null || true); \
	  sha=$$(git -C "$(WORKTREE_ROOT_ABS)" rev-parse --short HEAD 2>/dev/null || true); \
	  if [ -n "$$sha" ]; then echo "worker ref:    $${branch:-detached} @ $$sha"; else echo "worker ref:    unavailable"; fi
	@echo 'tmux windows:'
	@tmux list-windows -t "$(TMUX_SESSION)" -F '#I:#W #{pane_current_path} #{pane_current_command}' 2>/dev/null || \
	  echo "  no tmux session named $(TMUX_SESSION)"

# ---------------------------------------------------------------------------
# Harness conformance E2E (tech-specs/2026-07-15-harness-evaluation/conformance-e2e.md)
#
# Builds the subject stack + the standalone runner (evals/conformance, its own
# workspace) and runs every scenario against the pinned engine binary. The
# engine is never downloaded here: pass III_BIN (CI builds it from
# evals/conformance/engine.lock; locally see that file for the pinned source).
CONFORMANCE_WORKERS := queue iii-directory session-manager context-manager
CONFORMANCE_PROFILE ?= release
CONFORMANCE_FLAG    := $(if $(filter release,$(CONFORMANCE_PROFILE)),--release,)
CONFORMANCE_SCENARIO ?= all
CONFORMANCE_ARTIFACTS ?= $(REPO_ROOT)/target/conformance

conformance-e2e:
	@if [ -z "$(III_BIN)" ]; then \
	  echo "III_BIN is required: path to the pinned iii engine binary."; \
	  echo "Pinned source: harness/evals/conformance/engine.lock"; \
	  exit 3; \
	fi
	@for w in $(CONFORMANCE_WORKERS) harness; do \
	  echo "building $$w ($(CONFORMANCE_PROFILE))"; \
	  cargo build $(CONFORMANCE_FLAG) --manifest-path "$(REPO_ROOT)/$$w/Cargo.toml" || exit 1; \
	done
	@echo "building harness-conformance ($(CONFORMANCE_PROFILE))"
	@cargo build $(CONFORMANCE_FLAG) --manifest-path "$(MAKEFILE_DIR)evals/conformance/Cargo.toml"
	@"$(MAKEFILE_DIR)evals/conformance/target/$(CONFORMANCE_PROFILE)/harness-conformance" \
	  --engine-bin "$(III_BIN)" \
	  --harness-bin "$(REPO_ROOT)/harness/target/$(CONFORMANCE_PROFILE)/harness" \
	  --worker-bin "queue=$(REPO_ROOT)/queue/target/$(CONFORMANCE_PROFILE)/queue" \
	  --worker-bin "iii-directory=$(REPO_ROOT)/iii-directory/target/$(CONFORMANCE_PROFILE)/iii-directory" \
	  --worker-bin "session-manager=$(REPO_ROOT)/session-manager/target/$(CONFORMANCE_PROFILE)/session-manager" \
	  --worker-bin "context-manager=$(REPO_ROOT)/context-manager/target/$(CONFORMANCE_PROFILE)/context-manager" \
	  --scenario "$(CONFORMANCE_SCENARIO)" \
	  --artifacts-dir "$(CONFORMANCE_ARTIFACTS)"

$(STACK):
	@:
