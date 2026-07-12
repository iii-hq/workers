include $(dir $(abspath $(lastword $(MAKEFILE_LIST))))/Makefile.common

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
	@set -eu; \
	pid_file='$(PID_FILE)'; \
	repo_root='$(REPO_ROOT)'; \
	worker_root='$(WORKTREE_ROOT_ABS)'; \
	engine_config='$(ENGINE_CONFIG)'; \
	engine_url='$(ENGINE_URL)'; \
	engine_port='$(ENGINE_PORT)'; \
	console_port='$(CONSOLE_PORT)'; \
	rust_log='$(RUST_LOG)'; \
	run_profile='$(RUN_PROFILE)'; \
	stack='$(STACK)'; \
	is_alive() { kill -0 "$$1" 2>/dev/null; }; \
	if command -v iii >/dev/null 2>&1; then :; else echo "iii CLI is required; run make install-iii-next first" >&2; exit 127; fi; \
	if command -v cargo >/dev/null 2>&1; then :; else echo "cargo is required; install the Rust toolchain first" >&2; exit 127; fi; \
	if [ -f "$$pid_file" ]; then \
		alive_pid=; \
		while IFS=' ' read -r _name pid; do \
			case "$$pid" in ''|*[!0-9]*) continue ;; esac; \
			if is_alive "$$pid"; then alive_pid="$$pid"; break; fi; \
		done < "$$pid_file"; \
		if [ -n "$$alive_pid" ]; then echo "foreground harness stack is already running (pid $$alive_pid)" >&2; echo "run make dev-down before starting it again" >&2; exit 2; fi; \
		rm -f "$$pid_file"; \
	fi; \
	mkdir -p "$$(dirname "$$pid_file")"; \
	: > "$$pid_file"; \
	pids=; \
	record_pid() { name="$$1"; pid="$$2"; pids="$$pids $$pid"; printf '%s %s\\n' "$$name" "$$pid" >> "$$pid_file"; printf 'started %-24s (pid %s)\\n' "$$name" "$$pid"; }; \
	cleanup() { exit_code=$$?; trap - 0 1 2 3 15; for pid in $$pids; do kill "$$pid" 2>/dev/null || true; done; for pid in $$pids; do wait "$$pid" 2>/dev/null || true; done; rm -f "$$pid_file"; exit "$$exit_code"; }; \
	trap cleanup 0 1 2 3 15; \
	if command -v lsof >/dev/null 2>&1 && lsof -tiTCP:"$$engine_port" -sTCP:LISTEN >/dev/null 2>&1; then echo "TCP $$engine_port is already in use" >&2; lsof -nP -iTCP:"$$engine_port" -sTCP:LISTEN || true; exit 2; fi; \
	( cd "$$repo_root"; exec iii -c "$$engine_config" ) & \
	engine_pid=$$!; \
	record_pid engine "$$engine_pid"; \
	echo "waiting for engine on $$engine_url"; \
	i=0; \
	while [ "$$i" -lt 40 ]; do \
		if iii trigger engine::workers::list --port "$$engine_port" --json '{}' >/dev/null 2>&1; then echo "engine ready"; break; fi; \
		if ! is_alive "$$engine_pid"; then echo "engine exited before becoming ready" >&2; exit 1; fi; \
		i=$$((i + 1)); sleep 0.5; \
	done; \
	if [ "$$i" -ge 40 ]; then echo "engine did not become ready on $$engine_url" >&2; exit 1; fi; \
	start_worker() { \
		worker="$$1"; manifest="$$worker_root/$$worker/Cargo.toml"; \
		test -f "$$manifest" || { echo "missing $$manifest" >&2; return 1; }; \
		( cd "$$worker_root"; \
			if [ "$$worker" = console ]; then \
				if [ "$$run_profile" = release ]; then exec env RUST_LOG="$$rust_log" cargo run --release --manifest-path "$$manifest" -- --url "$$engine_url" --http-port "$$console_port"; fi; \
				exec env RUST_LOG="$$rust_log" cargo run --manifest-path "$$manifest" -- --url "$$engine_url" --http-port "$$console_port"; \
			fi; \
			if [ "$$run_profile" = release ]; then exec env RUST_LOG="$$rust_log" cargo run --release --manifest-path "$$manifest" -- --url "$$engine_url"; fi; \
			exec env RUST_LOG="$$rust_log" cargo run --manifest-path "$$manifest" -- --url "$$engine_url" \
		) & \
		record_pid "$$worker" "$$!"; \
	}; \
	for worker in $$stack; do start_worker "$$worker"; done; \
	echo "foreground harness stack is running"; \
	echo "console: http://127.0.0.1:$$console_port"; \
	echo "press Ctrl-C to stop the engine and all workers"; \
	while is_alive "$$engine_pid"; do sleep 1; done; \
	echo "engine exited" >&2; \
	exit 1

dev-down:
	@set -eu; \
	pid_file='$(PID_FILE)'; \
	if [ ! -f "$$pid_file" ]; then echo "no foreground harness stack is running"; exit 0; fi; \
	while IFS=' ' read -r name pid; do \
		case "$$pid" in ''|*[!0-9]*) continue ;; esac; \
		if kill -0 "$$pid" 2>/dev/null; then echo "stopping $$name ($$pid)"; kill "$$pid" 2>/dev/null || true; fi; \
	done < "$$pid_file"; \
	rm -f "$$pid_file"

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
