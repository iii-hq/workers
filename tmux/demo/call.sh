#!/bin/bash
# demo/call.sh <engine-port> <function> [json]: call a function on that engine,
# e.g. `demo/call.sh 49371 tmux::usage::read '{"refresh":true}'`.
set -euo pipefail
port=${1:?engine port}; fn=${2:?function id}; json=${3:-"{}"}
iii=${III_BIN:-$(command -v iiii || command -v iii)}
# A relative path (./bin/iii) means from here: made absolute before the cd.
if [[ $iii == */* && $iii != /* ]]; then iii=$PWD/$iii; fi
cd / && exec "$iii" call --engine "ws://127.0.0.1:$port" "$fn" --json "$json"
