#!/bin/bash
# demo/usage.sh <engine-port> set <token> key=value…   (e.g. demo-work week=91 five_hour=40)
# demo/usage.sh <engine-port> show
# Tokens: demo-work (default), demo-personal, demo-side-project; keys: five_hour,
# five_hour_resets, week, week_resets, week_opus, week_sonnet, status (200, 401, 429).
# The worker reads each account at most every 5 minutes (once a minute with
# tmux::usage::read refresh: true); restart the container to read at once.
set -euo pipefail
port=${1:?engine port}; shift
mock=${MOCK_PORT:-$((port + 10000))}
docker exec -u dev -e MOCK_PORT="$mock" "acct-demo-$port" python3 /opt/demo/mock_usage.py "$@"
