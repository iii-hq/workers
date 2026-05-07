# Local demo harness driver — registry-free.
#
# Same operations as scripts/demo.sh, fronted as Make targets. Run from the
# harness/ dir; targets that need the workspace root resolve it automatically.
#
# Usage:
#   make             # help
#   make all         # build + engine + start + verify
#   make build       # cargo build --release for harness + 16 dep workers
#   make engine      # start `iii --use-default-config` in background
#   make start       # spawn each worker as a background process
#   make verify      # call harness::status + models::list
#   make web         # vite dev server in tmux
#   make stop        # kill every spawned process + engine + tmux
#   make restart     # stop + engine + start + verify (no rebuild)
#   make logs W=harness    # tail $(DEMO_DIR)/logs/<worker>.log
#   make logs-harness      # same, kebab form for any worker
#
# Override DEMO_DIR or III engine URL via env or `make VAR=value`:
#   make all DEMO_DIR=/tmp/iii III_DEMO_ENGINE_URL=ws://1.2.3.4:49134

SHELL := bash
.SHELLFLAGS := -eu -o pipefail -c
MAKEFLAGS += --no-print-directory

# Resolve workspace root from this Makefile's location (harness/Makefile → ..).
HARNESS_DIR    := $(abspath $(dir $(lastword $(MAKEFILE_LIST))))
WORKERS_REPO   := $(abspath $(HARNESS_DIR)/..)
DEMO_DIR       ?= $(HOME)/iii-harness-demo
DEMO_ENGINE_WS ?= $(or $(III_DEMO_ENGINE_URL),ws://127.0.0.1:49134)
PIDS_DIR       := $(DEMO_DIR)/pids
LOGS_DIR       := $(DEMO_DIR)/logs

WORKERS := \
  turn-orchestrator provider-router \
  session-tree session-inbox \
  models-catalog hook-fanout policy-denylist \
  shell subagent \
  provider-anthropic provider-openai \
  auth-credentials llm-budget \
  skills approval-gate

# Resolve the binary name for a worker. Reads `bin:` from each crate's
# iii.worker.yaml (mixed: iii-* historical + folder-matched names). Falls
# back to the directory name if the YAML is missing.
define BIN_NAME_FOR
$$(awk '/^bin:/{sub(/^bin:[ \t]*/, ""); sub(/[ \t]+$$/, ""); print; exit}' "$(WORKERS_REPO)/$(1)/iii.worker.yaml" 2>/dev/null || echo "$(1)")
endef

.PHONY: help all build engine start verify web stop restart logs ensure-dirs
# logs-% is a pattern rule below; declaring each variant in .PHONY would
# block the pattern from firing.

help:
	@awk '/^[^#]/ && !/^$$/ {exit} /^#/ {sub(/^# ?/, ""); print}' $(firstword $(MAKEFILE_LIST))

all: build engine start verify

ensure-dirs:
	@mkdir -p $(PIDS_DIR) $(LOGS_DIR)

# ─── build ───────────────────────────────────────────────────────────────────

build: ensure-dirs
	@echo "==> building harness + $(words $(WORKERS)) dep workers (release)..."
	@for w in $(WORKERS) harness; do \
	  bin="$$(awk '/^bin:/{sub(/^bin:[ \t]*/, ""); sub(/[ \t]+$$/, ""); print; exit}' "$(WORKERS_REPO)/$$w/iii.worker.yaml" 2>/dev/null || echo "$$w")"; \
	  echo "    [build] $$w (bin=$$bin)"; \
	  ( cd "$(WORKERS_REPO)/$$w" && cargo build --release --bin "$$bin" --quiet ); \
	done
	@echo "==> all binaries built."

# ─── engine ──────────────────────────────────────────────────────────────────

engine: ensure-dirs
	@command -v iii >/dev/null || { echo "iii CLI not found"; exit 1; }
	@if [[ -f "$(DEMO_DIR)/engine.pid" ]] && kill -0 "$$(cat $(DEMO_DIR)/engine.pid)" 2>/dev/null; then \
	  echo "==> engine already running (pid $$(cat $(DEMO_DIR)/engine.pid))"; \
	  exit 0; \
	fi
	@echo "==> starting iii engine..."
	@cd "$(DEMO_DIR)" && { nohup iii --use-default-config > engine.log 2>&1 & echo $$! > "$(DEMO_DIR)/engine.pid"; }
	@for i in 1 2 3 4 5 6 7 8 9 10; do \
	  sleep 1; \
	  if iii --use-default-config trigger --function-id engine::queue::list_topics --timeout-ms 1000 >/dev/null 2>&1; then \
	    echo "==> engine ready after $${i}s (pid $$(cat $(DEMO_DIR)/engine.pid))"; \
	    exit 0; \
	  fi; \
	done; \
	echo "==> engine did not become ready in 10s; tail $(DEMO_DIR)/engine.log"; exit 1

# ─── start ───────────────────────────────────────────────────────────────────

start: ensure-dirs
	@command -v iii >/dev/null || { echo "iii CLI not found"; exit 1; }
	@echo "==> engine reachability check..."
	@iii --use-default-config trigger --function-id engine::queue::list_topics --timeout-ms 1000 >/dev/null 2>&1 \
	  || { echo "    engine not reachable — run \`make engine\` first (or use \`make all\`)"; exit 1; }
	@echo "==> spawning $(words $(WORKERS)) dep workers..."
	@for w in $(WORKERS); do $(MAKE) --no-print-directory _spawn-one W=$$w; sleep 0.1; done
	@echo "==> spawning harness..."
	@$(MAKE) --no-print-directory _spawn-one W=harness
	@echo "==> waiting 2s for registrations to settle..."
	@sleep 2
	@echo "==> done. PIDs in $(PIDS_DIR)  Logs in $(LOGS_DIR)"

# Internal: spawn a single worker by name. Caller passes W=<worker>.
.PHONY: _spawn-one
_spawn-one:
	@w="$(W)"; \
	bin_name="$$(awk '/^bin:/{sub(/^bin:[ \t]*/, ""); sub(/[ \t]+$$/, ""); print; exit}' "$(WORKERS_REPO)/$$w/iii.worker.yaml" 2>/dev/null || echo "$$w")"; \
	bin="$(WORKERS_REPO)/$$w/target/release/$$bin_name"; \
	pidfile="$(PIDS_DIR)/$$w.pid"; \
	logfile="$(LOGS_DIR)/$$w.log"; \
	if [[ -f "$$pidfile" ]] && kill -0 "$$(cat $$pidfile)" 2>/dev/null; then \
	  echo "    [skip]  $$w (already running, pid $$(cat $$pidfile))"; exit 0; \
	fi; \
	if [[ ! -x "$$bin" ]]; then \
	  echo "    [error] $$w binary missing — run \`make build\` first"; exit 1; \
	fi; \
	: > "$$logfile"; \
	if [[ "$$w" == "harness" ]]; then \
	  env III_URL="$(DEMO_ENGINE_WS)" nohup "$$bin" --config "$(WORKERS_REPO)/harness/config.yaml" --url "$(DEMO_ENGINE_WS)" >> "$$logfile" 2>&1 & \
	  echo $$! > "$$pidfile"; \
	else \
	  env III_URL="$(DEMO_ENGINE_WS)" nohup "$$bin" >> "$$logfile" 2>&1 & \
	  echo $$! > "$$pidfile"; \
	fi; \
	echo "    [start] $$w pid=$$(cat $$pidfile) log=$$logfile"

# ─── verify ──────────────────────────────────────────────────────────────────

verify:
	@command -v iii >/dev/null || { echo "iii CLI not found"; exit 1; }
	@echo "==> harness::status"
	@iii --use-default-config trigger --function-id harness::status
	@echo
	@echo "==> models::list (proves models-catalog connected)"
	@iii --use-default-config trigger --function-id models::list || true

# ─── logs ────────────────────────────────────────────────────────────────────

# `make logs W=<worker>` — defaults to harness.
W ?= harness
logs:
	@exec tail -f "$(LOGS_DIR)/$(W).log"

# `make logs-<worker>` — kebab form for any known worker.
logs-%:
	@exec tail -f "$(LOGS_DIR)/$*.log"

# ─── web ─────────────────────────────────────────────────────────────────────

web:
	@command -v npm  >/dev/null || { echo "npm not found; install Node.js first"; exit 1; }
	@command -v tmux >/dev/null || { echo "tmux not found; install tmux"; exit 1; }
	@if [[ ! -d "$(HARNESS_DIR)/web/node_modules" ]]; then \
	  echo "==> installing web deps..."; \
	  ( cd "$(HARNESS_DIR)/web" && npm install --no-fund --no-audit ); \
	fi
	@if tmux has-session -t harness-web 2>/dev/null; then \
	  echo "==> harness-web tmux session already running. Attach with: tmux attach -t harness-web"; \
	  exit 0; \
	fi
	@echo "==> starting vite (proxies /bridge → http://127.0.0.1:3111)"
	@tmux new-session -d -s harness-web -c "$(HARNESS_DIR)/web" "npm run dev"
	@echo "==> http://localhost:5173    (logs: tmux attach -t harness-web)"

# ─── stop ────────────────────────────────────────────────────────────────────

stop:
	@if [[ ! -d "$(PIDS_DIR)" ]]; then \
	  echo "no pid dir at $(PIDS_DIR); nothing to stop"; exit 0; \
	fi
	@echo "==> stopping spawned processes..."
	@for pidfile in $(PIDS_DIR)/*.pid; do \
	  [[ -f "$$pidfile" ]] || continue; \
	  w="$$(basename "$$pidfile" .pid)"; \
	  pid="$$(cat $$pidfile)"; \
	  if kill -0 "$$pid" 2>/dev/null; then \
	    kill -TERM "$$pid" 2>/dev/null && echo "    [stop]  $$w pid=$$pid"; \
	  else \
	    echo "    [gone]  $$w pid=$$pid"; \
	  fi; \
	  rm -f "$$pidfile"; \
	done
	@if [[ -f "$(DEMO_DIR)/engine.pid" ]]; then \
	  epid="$$(cat $(DEMO_DIR)/engine.pid)"; \
	  if kill -0 "$$epid" 2>/dev/null; then \
	    kill -TERM "$$epid" 2>/dev/null && echo "    [stop]  engine pid=$$epid"; \
	  fi; \
	  rm -f "$(DEMO_DIR)/engine.pid"; \
	fi
	@if command -v tmux >/dev/null && tmux has-session -t harness-web 2>/dev/null; then \
	  tmux kill-session -t harness-web && echo "    [stop]  harness-web tmux"; \
	fi
	@echo "==> done."

# ─── restart ─────────────────────────────────────────────────────────────────

restart: stop engine start verify
