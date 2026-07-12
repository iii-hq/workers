ROOT_DIR        := $(dir $(abspath $(lastword $(MAKEFILE_LIST))))
HARNESS_DIR     := $(ROOT_DIR)harness
MODE_MAKEFILE   := $(if $(filter tmux,$(MAKECMDGOALS)),Makefile.tmux,Makefile)

include $(HARNESS_DIR)/Makefile.common

SELECTED_WORKERS := $(filter $(STACK),$(MAKECMDGOALS))

.DEFAULT_GOAL := help

.PHONY: help tmux tmux-only dev-up dev-down dev-restart status smoke where \
	install-iii-next build install-local clean cargo-clean \
	engine wait-engine ensure-session require-engine restart restart-worker \
	stop stop-worker stop-work stop-engine logs attach $(STACK)

help:
	@printf '%s\n' \
	  'Workers local development:' \
	  '  make dev-up                         start the stack in this terminal' \
	  '  make tmux dev-up                    start the stack in tmux (optional)' \
	  '  make dev-down                       stop the selected development mode' \
	  '  make dev-restart                    restart the selected development mode' \
	  '  make status                         list registered workers' \
	  '  make smoke                          run basic engine/harness/router probes' \
	  '  make tmux restart harness            restart one worker in tmux' \
	  '  make tmux restart console harness   restart several workers in tmux' \
	  '  make tmux stop harness shell         stop selected tmux workers' \
	  '  make tmux logs                       attach to the tmux session' \
	  '  HARNESS_TMUX_SESSION=name make tmux dev-up' \
	  '' \
	  'The default foreground mode does not require tmux.'

tmux:

tmux-only:
	@if [ "$(MODE_MAKEFILE)" != "Makefile.tmux" ]; then \
	  echo "this target requires tmux mode; use make tmux <target>"; \
	  exit 2; \
	fi

dev-up:
	@$(MAKE) --no-print-directory -C "$(HARNESS_DIR)" -f "$(MODE_MAKEFILE)" dev-up

dev-down:
	@$(MAKE) --no-print-directory -C "$(HARNESS_DIR)" -f "$(MODE_MAKEFILE)" dev-down

dev-restart:
	@$(MAKE) --no-print-directory -C "$(HARNESS_DIR)" -f "$(MODE_MAKEFILE)" dev-restart

status:
	@$(MAKE) --no-print-directory -C "$(HARNESS_DIR)" -f "$(MODE_MAKEFILE)" status

smoke:
	@$(MAKE) --no-print-directory -C "$(HARNESS_DIR)" -f "$(MODE_MAKEFILE)" smoke

where:
	@$(MAKE) --no-print-directory -C "$(HARNESS_DIR)" -f "$(MODE_MAKEFILE)" where

install-iii-next:
	@$(MAKE) --no-print-directory -C "$(HARNESS_DIR)" -f "$(MODE_MAKEFILE)" install-iii-next

build:
	@$(MAKE) --no-print-directory -C "$(HARNESS_DIR)" -f "$(MODE_MAKEFILE)" build $(SELECTED_WORKERS)

install-local:
	@$(MAKE) --no-print-directory -C "$(HARNESS_DIR)" -f "$(MODE_MAKEFILE)" install-local $(SELECTED_WORKERS)

clean:
	@$(MAKE) --no-print-directory -C "$(HARNESS_DIR)" -f "$(MODE_MAKEFILE)" clean

cargo-clean:
	@$(MAKE) --no-print-directory -C "$(HARNESS_DIR)" -f "$(MODE_MAKEFILE)" cargo-clean $(SELECTED_WORKERS)

engine: tmux-only
	@$(MAKE) --no-print-directory -C "$(HARNESS_DIR)" -f "$(MODE_MAKEFILE)" engine

wait-engine: tmux-only
	@$(MAKE) --no-print-directory -C "$(HARNESS_DIR)" -f "$(MODE_MAKEFILE)" wait-engine

ensure-session: tmux-only
	@$(MAKE) --no-print-directory -C "$(HARNESS_DIR)" -f "$(MODE_MAKEFILE)" ensure-session

require-engine: tmux-only
	@$(MAKE) --no-print-directory -C "$(HARNESS_DIR)" -f "$(MODE_MAKEFILE)" require-engine

restart: tmux-only
	@$(MAKE) --no-print-directory -C "$(HARNESS_DIR)" -f "$(MODE_MAKEFILE)" restart $(SELECTED_WORKERS)

restart-worker: tmux-only
	@$(MAKE) --no-print-directory -C "$(HARNESS_DIR)" -f "$(MODE_MAKEFILE)" restart-worker

stop: tmux-only
	@$(MAKE) --no-print-directory -C "$(HARNESS_DIR)" -f "$(MODE_MAKEFILE)" stop $(SELECTED_WORKERS)

stop-worker: tmux-only
	@$(MAKE) --no-print-directory -C "$(HARNESS_DIR)" -f "$(MODE_MAKEFILE)" stop-worker

stop-work: tmux-only
	@$(MAKE) --no-print-directory -C "$(HARNESS_DIR)" -f "$(MODE_MAKEFILE)" stop-work

stop-engine: tmux-only
	@$(MAKE) --no-print-directory -C "$(HARNESS_DIR)" -f "$(MODE_MAKEFILE)" stop-engine

logs: tmux-only
	@$(MAKE) --no-print-directory -C "$(HARNESS_DIR)" -f "$(MODE_MAKEFILE)" logs

attach: tmux-only
	@$(MAKE) --no-print-directory -C "$(HARNESS_DIR)" -f "$(MODE_MAKEFILE)" attach

$(STACK):
	@:
