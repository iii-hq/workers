ROOT_DIR        := $(dir $(abspath $(lastword $(MAKEFILE_LIST))))
HARNESS_DIR     := $(ROOT_DIR)harness
MODE_MAKEFILE   := $(if $(filter tmux,$(MAKECMDGOALS)),Makefile.tmux,Makefile)

.DEFAULT_GOAL := help

.PHONY: help tmux dev-up dev-down dev-restart status smoke where install-iii-next

help:
	@printf '%s\n' \
	  'Workers local development:' \
	  '  make dev-up                         start the stack in this terminal' \
	  '  make tmux dev-up                    start the stack in tmux (optional)' \
	  '  make dev-down                       stop the selected development mode' \
	  '  make dev-restart                    restart the selected development mode' \
	  '  make status                         list registered workers' \
	  '  make smoke                          run basic engine/harness/router probes' \
	  '' \
	  'The default foreground mode does not require tmux.'

tmux:

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
