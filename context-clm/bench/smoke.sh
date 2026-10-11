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
echo "smoke ok"
