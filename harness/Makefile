include $(dir $(abspath $(lastword $(MAKEFILE_LIST))))/Makefile.common

FOREGROUND_MAKEFILE := $(MAKEFILE_DIR)Makefile

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
	state_dir='$(STATE_DIR)'; \
	repo_root='$(REPO_ROOT)'; \
	worker_root='$(WORKTREE_ROOT_ABS)'; \
	engine_config='$(ENGINE_CONFIG)'; \
	engine_url='$(ENGINE_URL)'; \
	engine_port='$(ENGINE_PORT)'; \
	console_port='$(CONSOLE_PORT)'; \
	rust_log='$(RUST_LOG)'; \
	run_profile='$(RUN_PROFILE)'; \
	stack='$(STACK)'; \
	stop_grace_steps='$(STOP_GRACE_STEPS)'; \
	is_alive() { kill -0 "$$1" 2>/dev/null; }; \
	if command -v iii >/dev/null 2>&1; then :; else echo "iii CLI is required; run make install-iii-next first" >&2; exit 127; fi; \
	if command -v cargo >/dev/null 2>&1; then :; else echo "cargo is required; install the Rust toolchain first" >&2; exit 127; fi; \
	case "$$state_dir" in ''|/) echo "invalid foreground state directory: $$state_dir" >&2; exit 2 ;; esac; \
	if ! mkdir "$$state_dir" 2>/dev/null; then \
		owner=$$(sed -n '1p' "$$state_dir/supervisor.pid" 2>/dev/null || true); \
		case "$$owner" in \
			''|*[!0-9]*) echo "foreground state exists without a valid owner; run make dev-down" >&2; exit 2 ;; \
		esac; \
		if is_alive "$$owner"; then echo "foreground harness stack is already running (supervisor $$owner)" >&2; exit 2; fi; \
		rm -f "$$state_dir/stop" "$$state_dir/processes" "$$state_dir/supervisor.pid"; \
		rmdir "$$state_dir" 2>/dev/null || { echo "cannot remove stale foreground state: $$state_dir" >&2; exit 2; }; \
		mkdir "$$state_dir" || { echo "cannot create foreground state: $$state_dir" >&2; exit 2; }; \
	fi; \
	pids=; \
	record_pid() { name="$$1"; pid="$$2"; pids="$$pids $$pid"; printf '%s %s\n' "$$name" "$$pid" >> "$$state_dir/processes"; printf 'started %-24s (pid %s)\n' "$$name" "$$pid"; }; \
	cleanup() { \
		exit_code=$$?; trap - 0 1 2 3 15; \
		for pid in $$pids; do if command -v pkill >/dev/null 2>&1; then pkill -TERM -P "$$pid" 2>/dev/null || true; fi; kill -TERM "$$pid" 2>/dev/null || true; done; \
		grace=0; \
		while [ "$$grace" -lt "$$stop_grace_steps" ]; do alive=0; for pid in $$pids; do if is_alive "$$pid"; then alive=1; fi; done; [ "$$alive" -eq 0 ] && break; grace=$$((grace + 1)); sleep 0.1; done; \
		for pid in $$pids; do if is_alive "$$pid"; then if command -v pkill >/dev/null 2>&1; then pkill -KILL -P "$$pid" 2>/dev/null || true; fi; kill -KILL "$$pid" 2>/dev/null || true; fi; done; \
		for pid in $$pids; do wait "$$pid" 2>/dev/null || true; done; \
		owner=$$(sed -n '1p' "$$state_dir/supervisor.pid" 2>/dev/null || true); \
		if [ "$$owner" = "$$$$" ]; then rm -f "$$state_dir/stop" "$$state_dir/processes" "$$state_dir/supervisor.pid"; rmdir "$$state_dir" 2>/dev/null || true; fi; \
		exit "$$exit_code"; \
	}; \
	trap cleanup 0 1 2 3 15; \
	printf '%s\n' "$$$$" > "$$state_dir/supervisor.pid"; \
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
	while [ ! -f "$$state_dir/stop" ]; do \
		exited=; \
		while IFS=' ' read -r name pid; do if ! is_alive "$$pid"; then exited="$$name ($$pid)"; break; fi; done < "$$state_dir/processes"; \
		if [ -n "$$exited" ]; then echo "$$exited exited; stopping foreground harness stack" >&2; exit 1; fi; \
		sleep 0.2; \
	done; \
	echo "stopping foreground harness stack"

dev-down:
	@set -eu; \
	state_dir='$(STATE_DIR)'; \
	stop_wait_steps='$(STOP_WAIT_STEPS)'; \
	case "$$state_dir" in ''|/) echo "invalid foreground state directory: $$state_dir" >&2; exit 2 ;; esac; \
	if [ ! -d "$$state_dir" ]; then echo "no foreground harness stack is running"; exit 0; fi; \
	i=0; \
	while [ ! -s "$$state_dir/supervisor.pid" ] && [ "$$i" -lt 20 ]; do i=$$((i + 1)); sleep 0.1; done; \
	owner=$$(sed -n '1p' "$$state_dir/supervisor.pid" 2>/dev/null || true); \
	case "$$owner" in \
		''|*[!0-9]*) rm -f "$$state_dir/stop" "$$state_dir/processes" "$$state_dir/supervisor.pid"; rmdir "$$state_dir" 2>/dev/null || true; echo "removed stale foreground state"; exit 0 ;; \
	esac; \
	if ! kill -0 "$$owner" 2>/dev/null; then rm -f "$$state_dir/stop" "$$state_dir/processes" "$$state_dir/supervisor.pid"; rmdir "$$state_dir" 2>/dev/null || true; echo "removed stale foreground state"; exit 0; fi; \
	: > "$$state_dir/stop"; \
	i=0; \
	while [ -d "$$state_dir" ] && [ "$$i" -lt "$$stop_wait_steps" ]; do i=$$((i + 1)); sleep 0.1; done; \
	if [ ! -d "$$state_dir" ]; then echo "foreground harness stack stopped"; exit 0; fi; \
	echo "foreground harness stack did not stop within $$(($$stop_wait_steps / 10)) seconds (supervisor $$owner)" >&2; \
	exit 1

dev-restart:
	@$(MAKE) --no-print-directory -f "$(FOREGROUND_MAKEFILE)" dev-down
	@$(MAKE) --no-print-directory -f "$(FOREGROUND_MAKEFILE)" dev-up

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
	@echo "state dir:     $(STATE_DIR)"
	@branch=$$(git -C "$(REPO_ROOT)" branch --show-current 2>/dev/null || true); \
	  sha=$$(git -C "$(REPO_ROOT)" rev-parse --short HEAD 2>/dev/null || true); \
	  if [ -n "$$sha" ]; then echo "repo ref:      $${branch:-detached} @ $$sha"; else echo "repo ref:      unavailable"; fi
	@branch=$$(git -C "$(WORKTREE_ROOT_ABS)" branch --show-current 2>/dev/null || true); \
	  sha=$$(git -C "$(WORKTREE_ROOT_ABS)" rev-parse --short HEAD 2>/dev/null || true); \
	  if [ -n "$$sha" ]; then echo "worker ref:    $${branch:-detached} @ $$sha"; else echo "worker ref:    unavailable"; fi

$(STACK):
	@:
