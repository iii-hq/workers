#!/usr/bin/env bash
# Configuration-only validation. Never starts an engine, model CLI, or external service.
# Run from this worktree's root after frozen Node dependency installation.
set -euo pipefail
ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-4}"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-/home/doggao/iii/iii/target}"
export ORT_LIB_PATH="${ORT_LIB_PATH:-/home/doggao/.cache/ort.pyke.io/dfbin/x86_64-unknown-linux-gnu/e454f710f8a49f53aa5b4ff51e3454ae1835777e431c6c35c5255ce6f205fd68}"
export ORT_PREFER_DYNAMIC_LINK=0
unset III_CONFIG_NAME
cargo test --manifest-path "$ROOT/crates/config-client/Cargo.toml" --locked --offline --lib
for worker in a2ui ade approval-gate bridge browser canvas code-runner codex computer \
  context-manager cron database devin document email github grok harness http ide \
  iii-directory memory memory-consolidate pdf provider-xai pubsub queue rbac-proxy \
  security-scan session-manager slack state storage tailscale telegram-bot voice \
  worktree fp web workflow sandbox-code-runner kanban; do
  printf '\n=== %s: configuration library tests ===\n' "$worker"
  timeout 120s cargo test --manifest-path "$ROOT/$worker/Cargo.toml" \
    --locked --offline --lib configuration::tests::
done
cargo test --manifest-path "$ROOT/ade/Cargo.toml" --locked --offline \
  --test configuration_delivery
cargo test --manifest-path "$ROOT/approval-gate/Cargo.toml" --locked --offline \
  --test repository_permissions
for worker in claude-code opencode pi vscode cursor; do
  pnpm --dir "$ROOT/$worker" exec vitest run \
    tests/configuration.test.ts tests/configuration-identity.test.ts
  pnpm --dir "$ROOT/$worker" exec biome check src/configuration.ts \
    tests/configuration-identity.test.ts
done
node --test "$ROOT/openwiki/tests/config.test.mjs" \
  "$ROOT/openwiki/tests/configuration-identity.test.mjs"
for worker in opencode vscode cursor; do
  pnpm --dir "$ROOT/$worker" exec tsc -p tsconfig.json --noEmit --incremental false
done
pnpm --dir "$ROOT/cursor" exec tsc -p tsconfig.test.json
# Claude Code/Pi full typechecking additionally needs their normal UI asset build.
# Published-engine validation is separate: an operator must supply the isolated
# 0.24.0 / 0.24.1+ endpoint and disposable namespace. This script starts neither.
