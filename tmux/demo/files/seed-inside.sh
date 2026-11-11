#!/bin/bash
# The artboards' sessions, made in this container's own tmux server the way
# the worker makes them (agent-accounts §3.5, D4): a placeholder first, the
# marks, then the program. Run as dev inside the demo container.
set -euo pipefail
# Only ever inside the demo container: it replaces the demo sessions of the
# tmux server it reaches.
if [[ ! -f /.dockerenv || ! -f /opt/demo/entrypoint.sh || "$(id -un)" != dev ]]; then
  echo "seed-inside.sh runs only inside the demo container" >&2; exit 2
fi
unset TMUX TMUX_PANE
P=/home/dev/projects/app
for s in agents review work; do tmux kill-session -t "=$s" 2>/dev/null || true; done
start() { # start <target> <cwd> <mark|-> <skip 0|1> <command…>
  local pane=$1 cwd=$2 mark=$3 skip=$4; shift 4
  local scope=-p; [[ "$(tmux -V)" =~ tmux\ [12]\. ]] && scope=-w
  [[ "$mark" != - ]] && tmux set-option $scope -t "$pane" @iii_claude_account "$mark"
  [[ "$skip" == 1 ]] && tmux set-option $scope -t "$pane" @iii_skip_permissions 1
  tmux respawn-pane -k -t "$pane" -c "$cwd" "$*"
}
pane_of() { tmux display -p -t "$1" '#{pane_id}'; }
tmux new-session -d -s agents -n claude -c "$P" -x 200 -y 50 cat
start "$(pane_of agents:0)" "$P" global 0 "FAKE_CLAUDE_TITLE='Fix the login flow on the settings page' claude"
tmux new-window -d -t agents: -n codex -c /home/dev/projects/api cat
start "$(pane_of agents:1)" /home/dev/projects/api - 0 "FAKE_CODEX_TITLE='Review PR 1444' codex"
tmux new-session -d -s review -n claude -c "$P" -x 200 -y 50 cat
# Pinned to Side project, or to the account the demo has instead (an
# unregistered folder would be taken, and "Add account" would then make
# side-project-2).
registry=/home/dev/.local/share/iii/claude-accounts/accounts.json
pin=default; on="env -u CLAUDE_CONFIG_DIR"
for id in side-project personal; do
  if [[ -f $registry ]] && grep -q "\"id\": \"$id\"" "$registry"; then
    pin=$id; on="env CLAUDE_CONFIG_DIR=/home/dev/.local/share/iii/claude-accounts/$id"; break
  fi
done
start "$(pane_of review:0)" "$P" "$pin" 1 "FAKE_CLAUDE_TITLE='Triage the flaky tests' $on claude --dangerously-skip-permissions"
tmux new-window -d -t review: -n logs -c "$P"
tmux new-session -d -s work -n editor -c "$P" -x 200 -y 50
tmux new-window -d -t work: -n claude -c "$P" cat
start "$(pane_of work:1)" "$P" global 0 "FAKE_CLAUDE_TITLE='Write the usage reader' claude"
tmux list-panes -a -F '#{session_name}:#{window_index} #{window_name} #{pane_id} #{@iii_claude_account}'
