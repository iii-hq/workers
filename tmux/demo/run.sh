#!/bin/bash
# demo/run.sh <engine-port> [--accounts all|two|one] [--fresh]
# Runs this branch's tmux worker in a container of its own (its own tmux
# server, fake Claude Code and Codex, fake accounts, the mock usage endpoint)
# against the iii engine on 127.0.0.1:<engine-port>. The container is
# acct-demo-<engine-port>; the worker binary is $WORKER_BIN (default: this
# checkout's target/release/tmux), mounted read-only, so restarting the
# container picks up a new build. --fresh drops the container's fake home.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
port=${1:?engine port, e.g. 49371}; shift
accounts=all; fresh=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --accounts) accounts=$2; shift 2;;
    --fresh) fresh=1; shift;;
    *) echo "unknown option $1" >&2; exit 2;;
  esac
done
bin=${WORKER_BIN:-$here/../target/release/tmux}
[[ -x "$bin" ]] || { echo "no worker binary at $bin (cargo build --release)" >&2; exit 1; }
image=${DEMO_IMAGE:-acct-demo:latest}
name=acct-demo-$port
mock=${MOCK_PORT:-$((port + 10000))}
if nc -z 127.0.0.1 "$mock" 2>/dev/null && ! docker ps --format '{{.Names}}' | grep -qx "$name"; then
  echo "port $mock (mock usage endpoint) is taken; set MOCK_PORT" >&2; exit 1
fi
docker image inspect "$image" >/dev/null 2>&1 || docker build -q -t "$image" "$here" >/dev/null
docker rm -f "$name" >/dev/null 2>&1 || true
[[ $fresh == 1 ]] && docker volume rm "$name-home" >/dev/null 2>&1 || true
tz=${TZ:-$(cat /etc/timezone 2>/dev/null || echo UTC)}
docker run -d --name "$name" --network host --label acct-demo-port="$port" \
  -e ENGINE_URL="ws://127.0.0.1:$port" -e MOCK_PORT="$mock" -e TZ="$tz" \
  -e DEMO_ACCOUNTS="$accounts" -e RUST_LOG="${RUST_LOG:-info}" \
  -v "$name-home:/home/dev" -v "$(realpath "$bin"):/w/tmux-worker:ro" \
  "$image" >/dev/null
echo "container $name: worker -> ws://127.0.0.1:$port, mock usage on 127.0.0.1:$mock"
echo "next: $here/seed.sh $port"
