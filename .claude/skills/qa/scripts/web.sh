#!/bin/zsh
# This branch's web app against the QA stack, with every worker UI that has a
# build served from this branch (the override key is the bundle scope).
#   web.sh            build worker UIs that changed since origin/main, start vite (detached)
#   QA_BUILD=all web.sh   rebuild every worker UI first
# Env: QA_NAME (qa) QA_WEB_PORT (5197) QA_ENGINE_PORT (49164) QA_CONSOLE_PORT (3143)
set -euo pipefail
REPO=$(git -C "$(dirname "$0")" rev-parse --show-toplevel)
PORT=${QA_WEB_PORT:-5197}
export PATH="$HOME/.local/bin:$PATH"

# scope=dir pairs; the scope is what /ui/<scope>/page.js is served under.
PAIRS=(console=ade browser=browser harness=harness kanban=kanban state=state cron=cron ide=ide
  learn-iii=onboarding sentinel=sentinel voice=voice a2ui=a2ui database=database
  iii-directory=iii-directory computer=computer judge=judge judge-typesafe=judge-typesafe
  llm-router=llm-router context-manager=context-manager)

changed=$(git -C "$REPO" diff --name-only origin/main... 2>/dev/null; git -C "$REPO" diff --name-only)
OV=()
for pair in $PAIRS; do
  scope=${pair%%=*}; dir=${pair#*=}
  [[ -f "$REPO/$dir/ui/package.json" ]] || continue
  if [[ "${QA_BUILD:-}" == all ]] || [[ ! -d "$REPO/$dir/ui/dist" ]] || echo "$changed" | grep -q "^$dir/ui/"; then
    echo "building $dir/ui"; (cd "$REPO/$dir/ui" && pnpm run build >/dev/null 2>&1) || echo "  BUILD FAILED: $dir/ui (report it)"
  fi
  [[ -d "$REPO/$dir/ui/dist" ]] && OV+=("$scope=../../$dir/ui/dist")
done

if lsof -iTCP:"$PORT" -sTCP:LISTEN >/dev/null 2>&1; then echo "port $PORT in use"; exit 1; fi
cd "$REPO/ade/web"
III_ENGINE_URL=ws://127.0.0.1:${QA_ENGINE_PORT:-49164} \
III_CONSOLE_URL=http://127.0.0.1:${QA_CONSOLE_PORT:-3143} \
III_WORKER_UI_OVERRIDES="${(j:,:)OV}" \
  nohup node node_modules/vite/bin/vite.js --port "$PORT" --strictPort \
  > "$REPO/.context/${QA_NAME:-qa}-stack/vite.log" 2>&1 < /dev/null &
echo "vite on http://localhost:$PORT/ (pid $!), log .context/${QA_NAME:-qa}-stack/vite.log"
