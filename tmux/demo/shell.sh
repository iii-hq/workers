#!/bin/bash
# demo/shell.sh <engine-port> [command…]: a shell (or a command) as dev in the
# demo container, e.g. `demo/shell.sh 49371 tmux ls`. Its tmux is the
# container's own server.
set -euo pipefail
port=${1:?engine port}; shift
if [[ $# -eq 0 ]]; then exec docker exec -it -u dev "acct-demo-$port" bash; fi
exec docker exec -u dev "acct-demo-$port" "$@"
