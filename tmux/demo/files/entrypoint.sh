#!/bin/bash
# Runs as `dev` in the demo container: fake accounts and Codex records (once),
# the mock usage endpoint on loopback, then the worker against the engine.
set -euo pipefail
# Only ever inside the demo container (demo/Dockerfile sets DEMO_CONTAINER=1):
# on a host this would write to the real home folder or talk to its tmux.
if [[ ! -f /.dockerenv || "${DEMO_CONTAINER:-}" != 1 ]]; then
  echo "$(basename "$0") runs only inside the demo container" >&2; exit 2
fi
: "${ENGINE_URL:?ENGINE_URL is required, e.g. ws://127.0.0.1:49370}"
: "${MOCK_PORT:?MOCK_PORT is required}"
export III_CLAUDE_USAGE_URL="http://127.0.0.1:${MOCK_PORT}"
mkdir -p /home/dev/demo /home/dev/projects/app /home/dev/projects/api
if [[ ! -f /home/dev/demo/.set-up ]]; then
  python3 /opt/demo/setup_demo.py --accounts "${DEMO_ACCOUNTS:-all}"
  touch /home/dev/demo/.set-up
fi
python3 /opt/demo/mock_usage.py serve --port "$MOCK_PORT" >/home/dev/demo/mock.log 2>&1 &
mock_up=0
for _ in $(seq 1 50); do
  curl -sf -o /dev/null "http://127.0.0.1:${MOCK_PORT}/state" && { mock_up=1; break; }
  sleep 0.1
done
if [[ $mock_up != 1 ]]; then
  echo "the mock usage endpoint did not start on :${MOCK_PORT}; see /home/dev/demo/mock.log" >&2
  exit 1
fi
# A neutral prompt for every shell the worker starts.
cat >/home/dev/.bashrc <<'RC'
PS1='\[\e[2m\]\w\[\e[0m\] $ '
RC
printf 'set -g status off\nset -g default-terminal xterm-256color\n' >/home/dev/.tmux.conf
exec /w/tmux-worker --url "$ENGINE_URL"
