#!/bin/bash
# demo/ask.sh <engine-port> <pane> [text]: type a line into the fake Claude
# Code in <pane> (e.g. %0), a request: it reads its sign-in again and
# answers "as <account>", so a switch of the global account shows in an
# idle session. `/login personal` signs that session's config in as Personal.
set -euo pipefail
port=${1:?engine port}; pane=${2:?pane, e.g. %0}; text=${3:-"What changed?"}
docker exec -u dev "acct-demo-$port" tmux send-keys -t "$pane" -l "$text"
docker exec -u dev "acct-demo-$port" tmux send-keys -t "$pane" Enter
