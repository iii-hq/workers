#!/bin/zsh
# What changed on this branch, in words a tester can turn into test
# charters: commit subjects (-v: and bodies), then the UI areas the files touch.
# File paths say WHERE to look; never read the code to decide what is right.
set -euo pipefail
REPO=$(git -C "$(dirname "$0")" rev-parse --show-toplevel)
VERBOSE=0; [[ "${1:-}" == -v ]] && { VERBOSE=1; shift; }
BASE=${1:-origin/main}
cd "$REPO"
echo "## Commits ($BASE..HEAD)"
if (( VERBOSE )); then
  git log --reverse --no-merges --format='- %s%n%w(0,4,4)%b' "$BASE"..HEAD | grep -v '^    Co-Authored-By' | sed '/^    $/d'
else
  git log --reverse --no-merges --format='- %s' "$BASE"..HEAD
fi
if [[ -n "$(git status --porcelain --untracked-files=no)" ]]; then
  echo "- (uncommitted changes present: test them too)"
fi
echo
echo "## Areas touched"
{ git diff --name-only "$BASE"...HEAD; git diff --name-only; } | sort -u | awk -F/ '
  $1 == "ade" && $2 == "web" { split($0, p, "/"); area = "ADE: " (p[4] == "components" ? p[5] : p[4]); }
  $1 == "ade" && $2 != "web" { area = "ADE worker: " $2 }
  $1 != "ade" && $2 == "ui" { area = "worker page: " $1 }
  $1 != "ade" && $2 != "ui" { area = "backend: " $1 }
  { n[area]++ }
  END { for (a in n) printf "- %s (%d files)\n", a, n[a] }' | sort
