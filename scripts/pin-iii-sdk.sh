#!/usr/bin/env bash
# Pin every project in this repository to one iii SDK release and re-lock it.
#
#   scripts/pin-iii-sdk.sh 0.26.0-rc.1 --dry-run   # print the edits and re-lock commands
#   scripts/pin-iii-sdk.sh 0.26.0-rc.1             # write the edits, re-lock, print a summary
#
# The last mile of an iii release, in order:
#
#   1. iii publishes the release. `Create Tag` in iii-hq/iii pushes iii/v<version>
#      (an rc from an unmerged branch: target iii, prerelease rc, version_override
#      0.26.0-rc.1; see .github/workflows/WORKFLOWS.md there), and release-iii.yml
#      publishes the SDK crates, the npm and PyPI packages, the Go module tag
#      sdk/packages/go/iii/v<version> and the engine archives on the GitHub release.
#   2. Run this script with that version on the workers branch. It stops before
#      writing anything while a registry does not serve the release yet.
#   3. Review `git diff`, commit and push the branch. CI then builds and tests every
#      worker against the published SDKs and the iii/v<version> engine.
#   4. Publish the worker releases through Release Control.
#   5. After the stable release (0.26.0), with the workers built on it published:
#        scripts/pin-iii-sdk.sh kit                       # deepwork kit ranges
#        scripts/pin-iii-sdk.sh template export <checkout of iii-hq/templates>
#        scripts/pin-iii-sdk.sh template record <its commit>
#
# What `pin` rewrites (VERSION in its spelling for each ecosystem):
#   rust    iii-sdk, iii-helpers, iii-observability, iii-filter and function-macros in
#           every Cargo.toml; then in every Cargo.lock that has them:
#           cargo update -p <crates>; cargo update -p <crate> --precise VERSION
#   node    iii-sdk, iii-browser-sdk, @iii-dev/helpers and @iii-dev/observability in
#           every package.json, the pnpm minimumReleaseAgeExclude entries, the
#           package.json a2ui generates; then pnpm install --lockfile-only (at the
#           pnpm version the project declares, --ignore-workspace for standalone
#           projects) or npm install --package-lock-only
#   python  iii-sdk, iii-helpers and iii-observability in every pyproject.toml (PEP 440,
#           0.26.0rc1), the sandbox-code-runner guest requirement; then
#           uv lock --upgrade-package <package>==VERSION
#   go      github.com/iii-hq/iii/sdk/packages/go/iii in every go.mod; then
#           GOWORK=off go mod tidy
#   engine  III_RELEASE_TAG and engine matrices in .github/workflows, the iii archive
#           URL and its amd64/arm64 sha256 in hermes/Dockerfile (read from the
#           release), and the tests that assert them
#
# Options: --only rust,node,python,go,engine  --project DIR  --no-relock
#          --no-preflight  --cargo-config FILE (a [patch] overlay, for example)
# Requires Python 3.9+, git, and the tools of the groups it re-locks: cargo, node
# with npx and npm, uv, go. `gh` or GITHUB_TOKEN reads the release.
set -euo pipefail

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)
exec python3 "$root/.github/scripts/pin_iii_sdk.py" "$@"
