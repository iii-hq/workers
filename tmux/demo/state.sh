#!/bin/bash
# demo/state.sh <engine-port> <pane> working|waiting|idle|limit [HH:MM]
# What the fake Claude Code in <pane> (e.g. %0) shows, and records in its
# sessions/<pid>.json.
set -euo pipefail
port=${1:?engine port}; shift
docker exec -u dev "acct-demo-$port" claude-state "$@"
