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
echo "smoke ok"
