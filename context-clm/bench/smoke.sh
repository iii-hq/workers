#!/usr/bin/env bash
# No-cost check of the bench stack wiring: boot, call the comparison arm directly (no model call), tear down.
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
: "${RUN:?set RUN}"
III=${III_BIN:-$HOME/.local/bin/iii}
trap '"$HERE/down.sh"' EXIT
"$HERE/boot.sh"
t() { "$III" trigger "$1" --engine ws://127.0.0.1:49250 --json "$2"; }
P='{"messages":[{"role":"user","content":[{"type":"text","text":"hi"}],"timestamp":1}],'
P+='"model":{"id":"claude-sonnet-5-5","provider":"anthropic"},"system_prompt":"sys",'
P+='"options":{"session_id":"smoke","turn_id":"t1","step":0,"budget_tokens":4096}}'
t context-base::assemble "$P" | jq -e '(.messages | length) == 1 and .usable > 0' >/dev/null
t context-base::status '{"session_id":"smoke"}' | jq -e '.stats.steps == 1 and .budget_tokens == 4096' >/dev/null
t context-sum::status '{"session_id":"smoke"}' | jq -e '.found == false' >/dev/null
test -s "$RUN/arms/window/smoke.txt"
# The stop path: a 2100-token budget leaves a 52-token limit, so step 0 gets the final turn and step 1 stops.
X=$(printf 'x%.0s' $(seq 4000))
S='{"messages":[{"role":"user","content":[{"type":"text","text":"'"$X"'"}],"timestamp":1}],'
S+='"model":{"id":"claude-sonnet-5-5","provider":"anthropic"},"system_prompt":"sys","options":{"session_id":"smoke-stop","turn_id":"t1",'
t context-base::assemble "$S"'"step":0,"budget_tokens":2100}}' | jq -e '.applied.strategy_stop == null' >/dev/null
t context-base::assemble "$S"'"step":1,"budget_tokens":2100}}' | jq -e '.applied.strategy_stop != null and .token_count > .usable' >/dev/null
# Execs find the iii shim first; it reaches this stack (context-base::status exists nowhere else) and nothing else.
SHIM=$(cd "$RUN" && pwd -P)/shim/iii
t shell::exec '{"command":"sh","args":["-c","command -v iii"]}' | jq -e --arg s "$SHIM" '.stdout == $s + "\n"' >/dev/null
"$SHIM" trigger context-base::status --json '{"session_id":"none"}' | jq -e '.found == false' >/dev/null
if "$SHIM" trigger --engine ws://127.0.0.1:1 engine::workers::list 2>/dev/null; then echo "shim let --engine through" >&2; exit 1; fi
# A scoped exec (as an episode's) writes cargo's download caches but not the rest of ~/.cargo or ~/.rustup.
PROBE='for p in ~/.cargo/.bench-probe ~/.rustup/.bench-probe; do touch "$p" 2>/dev/null && { rm -f "$p"; exit 1; }; done
touch ~/.cargo/registry/.bench-probe && rm ~/.cargo/registry/.bench-probe'
t shell::exec "$(jq -nc --arg s "$PROBE" --arg r "$(cd "$RUN" && pwd -P)/workspace" \
  '{command: "sh", args: ["-c", $s], fs_scope: {root: $r, boundary: "workspace"}}')" | jq -e '.exit_code == 0' >/dev/null
echo "smoke ok"
