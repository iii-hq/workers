#!/usr/bin/env bash
# Real end-to-end usage of the demo harness.
#
# Important: turn-orchestrator and session-tree::* use DIFFERENT stores.
#   - run::start_and_wait persists to engine state under scope="agent",
#     key="session/<id>/messages" (and a few sibling keys).
#   - session-tree::tree / session-tree::messages read session-tree::*'s own
#     store, populated only by explicit session-tree::create + append calls.
# This script uses the run::start path. To enumerate, we list state keys
# under scope="agent".
#
# Steps:
#   1. authenticate              — auth::set_token
#   2. send + wait               — run::start_and_wait (returns full transcript)
#   3. read messages from state  — state::get  scope=agent, key=session/<id>/messages
#   4. read the turn record      — state::get  scope=agent, key=session/<id>/turn_state
#   5. list every session        — state::list scope=agent, then filter
#
# Prereqs:
#   - Engine + 23 dep workers + iii-harness running.
#       ./harness/scripts/demo.sh all
#   - ANTHROPIC_API_KEY env var set (use a fresh, rotated key).
#       export ANTHROPIC_API_KEY='sk-ant-...'
#
# Usage:
#   ./harness/scripts/real-usage.sh                           # session_id=demo-real-1
#   ./harness/scripts/real-usage.sh my-session-id
#   ./harness/scripts/real-usage.sh my-session-id "build me a haiku"

set -euo pipefail

SID="${1:-demo-real-1}"
PROMPT="${2:-say hello in three words}"
MODEL="${MODEL:-claude-opus-4-7}"

if [[ -z "${ANTHROPIC_API_KEY:-}" ]]; then
  echo "ANTHROPIC_API_KEY is not set. export a fresh, rotated key first."
  exit 1
fi

iii_call() {
  iii --use-default-config trigger --function-id "$1" --payload "$2" --timeout-ms "${3:-30000}"
}

echo "==> 1/5 authenticate (auth::set_token, provider=anthropic)"
iii_call auth::set_token "$(cat <<JSON
{
  "provider": "anthropic",
  "credential": { "type": "api_key", "key": "$ANTHROPIC_API_KEY" }
}
JSON
)"
echo

echo "==> 2/5 send + wait (run::start_and_wait, session_id=$SID)"
echo "    response includes the full transcript: [user, assistant]"
iii_call run::start_and_wait "$(cat <<JSON
{
  "session_id": "$SID",
  "provider": "anthropic",
  "model": "$MODEL",
  "messages": [
    {
      "role": "user",
      "content": [{"type": "text", "text": "$PROMPT"}],
      "timestamp": 0
    }
  ]
}
JSON
)" 60000
echo

echo "==> 3/5 messages persisted to state (state::get session/$SID/messages)"
iii_call state::get "{\"scope\":\"agent\",\"key\":\"session/$SID/messages\"}"
echo

echo "==> 4/5 turn record (state::get session/$SID/turn_state)"
iii_call state::get "{\"scope\":\"agent\",\"key\":\"session/$SID/turn_state\"}"
echo

echo "==> 5/5 list every session this engine has seen"
echo "    state::list returns values without keys, so we filter for turn_state"
echo "    objects (the only ones that carry session_id + state fields)"
iii_call state::list '{"scope":"agent","prefix":"session/"}' \
  | python3 -c '
import sys, json
values = json.load(sys.stdin)
if not isinstance(values, list):
    values = []
sessions = []
for v in values:
    if isinstance(v, dict) and "session_id" in v and "state" in v:
        sessions.append({
            "session_id": v["session_id"],
            "state": v["state"],
            "turn_count": v.get("turn_count"),
            "updated_at_ms": v.get("updated_at_ms"),
        })
sessions.sort(key=lambda s: s.get("updated_at_ms") or 0, reverse=True)
print(json.dumps({"sessions": sessions, "count": len(sessions)}, indent=2))
'
echo

echo "==> done. session_id=$SID"
echo "    audit log: tail -f ~/.harness/audit.jsonl"
