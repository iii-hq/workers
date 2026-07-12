include $(dir $(abspath $(lastword $(MAKEFILE_LIST))))/Makefile.common

TMUX_MAKEFILE := $(MAKEFILE_DIR)Makefile.tmux

GOAL_ARGS        := $(filter-out help install-iii-next build install-local clean cargo-clean dev-up dev-down dev-restart \
                    engine wait-engine restart restart-worker stop stop-worker stop-work stop-engine status smoke logs attach \
                    ensure-session require-engine where,$(MAKECMDGOALS))
SELECTED_WORKERS := $(strip $(GOAL_ARGS))
ACTIVE_WORKERS   := $(if $(SELECTED_WORKERS),$(SELECTED_WORKERS),$(STACK))
UNKNOWN_WORKERS  := $(filter-out $(STACK),$(SELECTED_WORKERS))

.DEFAULT_GOAL := help

.PHONY: help install-iii-next build install-local clean cargo-clean dev-up dev-down dev-restart engine wait-engine \
        restart restart-worker stop stop-worker stop-work stop-engine status smoke logs attach ensure-session require-engine where $(STACK)

help:
	@printf '%s\n' \
	  'Harness tmux development targets (run from the repository root):' \
	  '  make tmux dev-up                    start engine + harness stack in tmux' \
	  '  make tmux dev-down                  stop the tmux stack' \
	  '  make tmux dev-restart               restart all workers in the stack' \
	  '  make tmux restart console harness shell' \
	  '  make tmux restart harness WORKTREE_ROOT=.worktrees/my-feature' \
	  '  make tmux stop harness shell' \
	  '  make tmux stop-work                 stop all worker windows, keep engine' \
	  '  make tmux stop-engine               stop the engine window only' \
	  '  make tmux status                    list registered workers' \
	  '  make tmux smoke                     run basic engine/harness/router probes' \
	  '  make tmux where                     print source roots, refs, and tmux panes' \
	  '  make tmux logs                      attach to the tmux session' \
	  '  HARNESS_TMUX_SESSION=name make tmux dev-up'

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
	@$(MAKE) --no-print-directory -f "$(TMUX_MAKEFILE)" restart $(STACK)
	@$(MAKE) --no-print-directory -f "$(TMUX_MAKEFILE)" status
	@echo "console: http://127.0.0.1:$(CONSOLE_PORT)"
	@echo "tmux:    tmux attach -t $(TMUX_SESSION)"

dev-down:
	@tmux kill-session -t "$(TMUX_SESSION)" 2>/dev/null || true

dev-restart:
	@$(MAKE) --no-print-directory -f "$(TMUX_MAKEFILE)" restart $(STACK)

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
		echo "run make tmux dev-up or make -f Makefile.tmux engine first"; \
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
	  manifest="$(WORKTREE_ROOT_ABS)/$$w/Cargo.toml"; \
	  test -f "$$manifest" || { echo "missing $$manifest"; exit 2; }; \
	done
	@$(MAKE) --no-print-directory -f "$(TMUX_MAKEFILE)" require-engine
	@$(MAKE) --no-print-directory -f "$(TMUX_MAKEFILE)" ensure-session
	@for w in $(SELECTED_WORKERS); do \
	  $(MAKE) --no-print-directory -f "$(TMUX_MAKEFILE)" restart-worker WORKER=$$w || exit 1; \
	done

restart-worker:
	@test -n "$(WORKER)" || { echo "WORKER is required"; exit 2; }
	@case " $(STACK) " in \
	  *" $(WORKER) "*) ;; \
	  *) echo "unknown worker: $(WORKER)"; echo "valid workers: $(STACK)"; exit 2 ;; \
	esac
	@worker="$(WORKER)"; \
	manifest="$(WORKTREE_ROOT_ABS)/$$worker/Cargo.toml"; \
	test -f "$$manifest" || { echo "missing $$manifest"; exit 2; }; \
	args="--url $(ENGINE_URL)"; \
	if [ "$$worker" = "console" ]; then args="$$args --http-port $(CONSOLE_PORT)"; fi; \
	cmd="cd '$(WORKTREE_ROOT_ABS)' && RUST_LOG='$(RUST_LOG)' cargo run $(RUN_FLAG) --manifest-path '$$manifest' -- $$args"; \
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
	  $(MAKE) --no-print-directory -f "$(TMUX_MAKEFILE)" stop-worker WORKER=$$w || exit 1; \
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
	  $(MAKE) --no-print-directory -f "$(TMUX_MAKEFILE)" stop-worker WORKER=$$w || exit 1; \
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

$(STACK):
	@:
