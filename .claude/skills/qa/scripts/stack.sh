#!/bin/zsh
# A throwaway stack exactly as a NEW USER gets it: the repo's template/
# (compose file, agent profiles, config, skills) with every worker built from
# THIS branch, empty data, on its own namespace and ports so it never touches
# the person's usual stack.
#
#   stack.sh up       render (if missing) and start, detached; log in $DIR/compose.log
#   stack.sh reset    stop, wipe data/ and the render: the next `up` is a brand-new user
#   stack.sh down     stop
#   stack.sh status   ports and pid
#
# Env: QA_NAME (qa: a second run alongside uses another name AND other ports)
#      QA_ENGINE_PORT (49164) QA_CONSOLE_PORT (3143) QA_SKIP (comma list of
# containers to leave out, e.g. "sandbox-libkrun,computer" when the VM image
# is not wanted) III_BIN (rc engine dir) CARGO_TARGET_DIR (shared build cache).
set -euo pipefail
REPO=$(git -C "$(dirname "$0")" rev-parse --show-toplevel)
NAME=${QA_NAME:-qa}
DIR="$REPO/.context/$NAME-stack"
NS="$(basename "$REPO")-$NAME"
ENGINE_PORT=${QA_ENGINE_PORT:-49164}
CONSOLE_PORT=${QA_CONSOLE_PORT:-3143}
III_BIN=${III_BIN:-$(ls -d ~/.local/iii-*/bin 2>/dev/null | sort -V | tail -1)}
export PATH="$III_BIN:$HOME/.cargo/bin:$HOME/.local/bin:$PATH"
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$REPO/.context/cargo-target}
export III_ENGINE_PORT=$ENGINE_PORT

render() {
  mkdir -p "$DIR"
  # Everything a new user downloads, minus the scripts that fetch upstream.
  rsync -a --delete --exclude data --exclude compose.log --exclude compose.pid \
    --exclude sync.sh --exclude upstream --exclude tests "$REPO/template/" "$DIR/"
  # Workers from this branch, wherever the render lives; our own namespace.
  sed -i '' -e "s#path://\.\./#path://$REPO/#g" -e "s#^namespace: .*#namespace: $NS#" \
    "$DIR/worker-compose.yaml"
  sed -i '' -e "s#^http_port: .*#http_port: $CONSOLE_PORT#" "$DIR/config/console.yaml"
  if [[ -n "${QA_SKIP:-}" ]]; then
    for c in ${(s:,:)QA_SKIP}; do
      # Drop the container block (its key line through the line before the next key).
      awk -v c="  $c:" '
        $0 == c { skip = 1; next }
        skip && /^  [A-Za-z0-9_-]+:$/ { skip = 0 }
        !skip' "$DIR/worker-compose.yaml" > "$DIR/wc.tmp" && mv "$DIR/wc.tmp" "$DIR/worker-compose.yaml"
      sed -i '' -e "/^      - $c\$/d" "$DIR/worker-compose.yaml"
    done
  fi
  mkdir -p "$DIR/data"
  echo "rendered $DIR (namespace $NS, engine $ENGINE_PORT, console $CONSOLE_PORT)"
}

stop() {
  if [[ -f "$DIR/compose.pid" ]] && kill -0 "$(cat "$DIR/compose.pid")" 2>/dev/null; then
    kill -INT "$(cat "$DIR/compose.pid")" || true
    for _ in {1..30}; do kill -0 "$(cat "$DIR/compose.pid")" 2>/dev/null || break; sleep 1; done
  fi
  rm -f "$DIR/compose.pid"
}

case "${1:-status}" in
  up)
    [[ -f "$DIR/worker-compose.yaml" ]] || render
    if lsof -iTCP:"$ENGINE_PORT" -sTCP:LISTEN >/dev/null 2>&1; then
      echo "engine port $ENGINE_PORT already in use"; exit 1
    fi
    cd "$DIR"
    nohup iii compose --up -f worker-compose.yaml > compose.log 2>&1 < /dev/null &
    echo $! > compose.pid
    echo "starting (pid $(cat compose.pid)); first build can take minutes: tail -f $DIR/compose.log"
    ;;
  reset) stop; rm -rf "$DIR"; render ;;
  down) stop; echo stopped ;;
  status)
    echo "dir $DIR  namespace $NS"
    for p in $ENGINE_PORT $CONSOLE_PORT; do
      lsof -iTCP:$p -sTCP:LISTEN -nP 2>/dev/null | awk 'NR>1{print "  listening", $1, $9}' || true
    done
    [[ -f "$DIR/compose.pid" ]] && echo "  compose pid $(cat "$DIR/compose.pid")" || true
    ;;
  *) echo "usage: stack.sh up|reset|down|status"; exit 2 ;;
esac
