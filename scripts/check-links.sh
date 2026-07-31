#!/usr/bin/env bash
# Check every iii.dev URL in the repo. Usage: scripts/check-links.sh
set -uo pipefail
cd "$(dirname "$0")/.."

# Known-good URLs that answer 404 to a plain GET; not broken doc links.
ignore=(
  'https://api.workers.iii.dev'            # API base, only POST routes
  'https://api.workers.iii.dev/workers'    # ditto
  'https://workers.iii.dev/workers/'       # URL prefix in a test assertion, not a link
  'https://workers.iii.dev/workers/skills' # published (badge.svg 200) but registry page missing
)

urls=$(grep -rhoE 'https?://[a-zA-Z0-9.-]*iii\.dev[^)"'"'"'`[:space:]>,]*' . \
  --exclude-dir=node_modules --exclude-dir=.git --exclude-dir=target --exclude-dir=dist 2>/dev/null \
  | sed 's/[.;:}]*$//' | sort -u)

fail=0
for url in $urls; do
  for skip in "${ignore[@]}"; do [[ "$url" == "$skip" ]] && continue 2; done
  for attempt in 1 2 3; do
    code=$(curl -sS -o /dev/null -w '%{http_code}' -L --max-time 20 "$url")
    [[ "$code" =~ ^(429|5..|000)$ ]] || break
    (( attempt < 3 )) && sleep $((attempt * attempt * 2))   # backoff: 2s, 8s
  done
  if [[ "$code" =~ ^[23] ]]; then
    printf 'OK   %s  %s\n' "$code" "$url"
  else
    printf 'FAIL %s  %s\n' "$code" "$url"
    grep -rl --fixed-strings "${url%%#*}" . \
      --exclude-dir=node_modules --exclude-dir=.git --exclude-dir=target --exclude-dir=dist 2>/dev/null \
      | sed 's/^/       ↳ /'
    fail=1
  fi
done
exit $fail
