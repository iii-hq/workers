# Local demo harness driver — playground-style.
#
# Workers and their per-worker config live in `harness/config.yaml`, the
# single source of truth the iii engine reads. Adding or removing a worker
# is a YAML edit, not a Makefile change. The engine spawns each worker
# from its `iii.worker.yaml` `scripts.start` (cargo run for local Rust
# crates — builds on demand).
#
# Usage:
#   make                # help
#   make all            # engine + verify
#   make engine         # start `iii` in background reading harness/config.yaml
#   make verify         # call harness::status + models::list
#   make web            # background vite dev server on :5173 (no tmux)
#   make stop           # kill engine + web
#   make restart        # stop + engine + verify
#   make logs W=harness # tail engine/web logs
#   make lock           # refresh harness/iii.lock via `iii worker add .`
#   make clean          # remove $(DEMO_DIR) state (pids, logs, data)
#
# Override DEMO_DIR or engine URL via env or `make VAR=value`:
#   make all DEMO_DIR=/tmp/iii III_DEMO_ENGINE_URL=ws://1.2.3.4:49134

SHELL := bash
.SHELLFLAGS := -eu -o pipefail -c
MAKEFLAGS += --no-print-directory

HARNESS_DIR    := $(abspath $(dir $(lastword $(MAKEFILE_LIST))))
DEMO_DIR       ?= $(HOME)/iii-harness-demo
DEMO_ENGINE_WS ?= $(or $(III_DEMO_ENGINE_URL),ws://127.0.0.1:49134)
PIDS_DIR       := $(DEMO_DIR)/pids
LOGS_DIR       := $(DEMO_DIR)/logs

.PHONY: help all engine verify web stop restart logs lock clean ensure-dirs

help:
	@awk '/^[^#]/ && !/^$$/ {exit} /^#/ {sub(/^# ?/, ""); print}' $(firstword $(MAKEFILE_LIST))

all: engine verify

ensure-dirs:
	@mkdir -p $(PIDS_DIR) $(LOGS_DIR) $(HARNESS_DIR)/data/skills

# ─── engine ──────────────────────────────────────────────────────────────────

engine: ensure-dirs
	@command -v iii >/dev/null || { echo "iii CLI not found"; exit 1; }
	@if [[ -f "$(PIDS_DIR)/engine.pid" ]] && kill -0 "$$(cat $(PIDS_DIR)/engine.pid)" 2>/dev/null; then \
	  echo "==> engine already running (pid $$(cat $(PIDS_DIR)/engine.pid))"; \
	  exit 0; \
	fi
	@echo "==> starting iii engine (cwd=$(HARNESS_DIR))..."
	@cd "$(HARNESS_DIR)" && { nohup iii --config config.yaml > "$(LOGS_DIR)/engine.log" 2>&1 & echo $$! > "$(PIDS_DIR)/engine.pid"; }
	@for i in 1 2 3 4 5 6 7 8 9 10; do \
	  sleep 1; \
	  if iii trigger --function-id engine::health::check --timeout-ms 1000 >/dev/null 2>&1; then \
	    echo "==> engine ready after $${i}s (pid $$(cat $(PIDS_DIR)/engine.pid))"; \
	    exit 0; \
	  fi; \
	done; \
	echo "==> engine did not become ready in 10s; tail $(LOGS_DIR)/engine.log"; exit 1

# ─── verify ──────────────────────────────────────────────────────────────────

verify:
	@command -v iii >/dev/null || { echo "iii CLI not found"; exit 1; }
	@echo "==> harness::status"
	@iii trigger --function-id harness::status
	@echo
	@echo "==> models::list (proves models-catalog connected)"
	@iii trigger --function-id models::list || true

# ─── web ─────────────────────────────────────────────────────────────────────

web: ensure-dirs
	@command -v npm >/dev/null || { echo "npm not found; install Node.js first"; exit 1; }
	@if [[ -f "$(PIDS_DIR)/web.pid" ]] && kill -0 "$$(cat $(PIDS_DIR)/web.pid)" 2>/dev/null; then \
	  echo "==> web already running (pid $$(cat $(PIDS_DIR)/web.pid))    http://localhost:5173"; \
	  exit 0; \
	fi
	@if [[ ! -d "$(HARNESS_DIR)/web/node_modules" ]]; then \
	  echo "==> installing web deps..."; \
	  ( cd "$(HARNESS_DIR)/web" && npm install --no-fund --no-audit ); \
	fi
	@echo "==> starting vite (proxies /bridge → http://127.0.0.1:3111)"
	@cd "$(HARNESS_DIR)/web" && { nohup npm run dev > "$(LOGS_DIR)/web.log" 2>&1 & echo $$! > "$(PIDS_DIR)/web.pid"; }
	@echo "==> http://localhost:5173    (logs: make logs W=web)"

# ─── logs ────────────────────────────────────────────────────────────────────

# `make logs` defaults to engine. `make logs W=web` tails the vite log.
# Engine-managed workers all log into engine.log; use the engine log for them.
W ?= engine
logs:
	@case "$(W)" in \
	  engine|web) exec tail -f "$(LOGS_DIR)/$(W).log" ;; \
	  *) echo "==> tailing engine.log (filter for '$(W)')"; exec tail -f "$(LOGS_DIR)/engine.log" ;; \
	esac

# ─── lock ────────────────────────────────────────────────────────────────────

# Refresh harness/iii.lock after editing iii.worker.yaml `dependencies:`.
# `iii worker add .` re-resolves the dep graph, preserves existing inline
# config blocks, and updates the lockfile.
lock:
	@command -v iii >/dev/null || { echo "iii CLI not found"; exit 1; }
	@cd "$(HARNESS_DIR)" && iii worker add .

# ─── stop ────────────────────────────────────────────────────────────────────

stop:
	@if [[ ! -d "$(PIDS_DIR)" ]]; then \
	  echo "no pid dir at $(PIDS_DIR); nothing to stop"; exit 0; \
	fi
	@for pidfile in $(PIDS_DIR)/*.pid; do \
	  [[ -f "$$pidfile" ]] || continue; \
	  name="$$(basename "$$pidfile" .pid)"; \
	  pid="$$(cat $$pidfile)"; \
	  if kill -0 "$$pid" 2>/dev/null; then \
	    kill -TERM "$$pid" 2>/dev/null && echo "    [stop]  $$name pid=$$pid"; \
	  else \
	    echo "    [gone]  $$name pid=$$pid"; \
	  fi; \
	  rm -f "$$pidfile"; \
	done

# ─── restart ─────────────────────────────────────────────────────────────────

restart: stop engine verify

# ─── clean ───────────────────────────────────────────────────────────────────

clean: stop
	@rm -rf "$(DEMO_DIR)" "$(HARNESS_DIR)/data"
	@echo "==> cleaned $(DEMO_DIR) and $(HARNESS_DIR)/data"
