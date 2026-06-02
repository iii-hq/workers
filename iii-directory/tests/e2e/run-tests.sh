#!/usr/bin/env bash
# End-to-end test for the iii-directory worker against REAL workers on
# https://api.workers.iii.dev. Builds + installs the worker, generates an
# absolute-path engine config from ./config.yaml, starts its own engine,
# downloads real bundles, and ASSERTS every behavior. Exits 0 on all pass,
# 1 otherwise.
#
#   ./run-tests.sh            # full run (builds + installs the worker first)
#   ./run-tests.sh --no-build # reuse the iii-directory already in ~/.iii/workers
#   ./run-tests.sh --keep     # leave the engine running afterwards
#   PORT=49210 ./run-tests.sh # use a non-default engine port
set -uo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"; cd "$ROOT_DIR"
HERE="$ROOT_DIR"                                   # assertion body refers to $HERE / $GLOBAL
PORT="${PORT:-49134}"
# Worker source is two levels up: iii-directory/tests/e2e -> iii-directory.
WORKER_SRC="${WORKER_SRC:-$(cd "$ROOT_DIR/../.." && pwd)}"
# Prefer iii on PATH, then the conventional install dir, then a local build.
III="${III:-$(command -v iii 2>/dev/null \
  || { [ -x "$HOME/.local/bin/iii" ] && echo "$HOME/.local/bin/iii"; } \
  || echo /Users/andersonleal/projetos/motia/motia/target/release/iii)}"
WORKERS_DIR="$HOME/.iii/workers"
GLOBAL="$ROOT_DIR/skills-home"                      # registry downloads land here
LOCAL="$ROOT_DIR/.iii/skills"                       # local-override fixtures
REPORTS="$ROOT_DIR/reports"; mkdir -p "$REPORTS"
ENGINE_CONFIG="$REPORTS/engine-config.yaml"         # generated below (gitignored)
ENGINE_LOG="$REPORTS/engine.log"

BUILD=1; KEEP=0
for a in "$@"; do case "$a" in
  --no-build) BUILD=0 ;;
  --keep)     KEEP=1 ;;
  -h|--help)  printf 'Usage: ./run-tests.sh [--no-build] [--keep]   (PORT=NNNN overrides the engine port)\n'; exit 0 ;;
  *) echo "unknown arg: $a"; exit 2 ;;
esac; done

PASS=0; FAIL=0; FAILED=()
ok() { PASS=$((PASS + 1)); printf '  \033[32m✓\033[0m %s\n' "$1"; }
no() { FAIL=$((FAIL + 1)); FAILED+=("$1"); printf '  \033[31m✗ %s\033[0m\n' "$1"; [ -n "${2:-}" ] && printf '      %s\n' "$(printf '%s' "$2" | head -c 200)"; }

TIMEOUT_BIN="$(command -v timeout || command -v gtimeout || true)"          # so one hung trigger can't block the whole suite
trig()  { ${TIMEOUT_BIN:+$TIMEOUT_BIN ${TRIG_TIMEOUT:-90}} "$III" trigger --port "$PORT" "$@" 2>&1; }   # raw (JSON on success, "Error: …" on failure)
jtrue() { if printf '%s' "$2" | jq -e "$3" >/dev/null 2>&1; then ok "$1"; else no "$1" "$2"; fi; }   # jq filter must eval truthy
has()   { case "$2" in *"$3"*) ok "$1" ;; *) no "$1" "missing «$3» in: $2" ;; esac; }
hasnt() { case "$2" in *"$3"*) no "$1" "should NOT contain «$3»" ;; *) ok "$1" ;; esac; }
iserr() { case "$2" in *Error:*|*'"type":"not_found"'*|*'invocation_failed'*) ok "$1" ;; *) no "$1" "expected an error, got: $2" ;; esac; }

# ── setup ───────────────────────────────────────────────────────────────────
echo "==> setup"
command -v jq >/dev/null 2>&1 || { echo "jq is required (brew install jq)"; exit 1; }
if [ "$BUILD" = 1 ]; then
  echo "    build + install iii-directory into $WORKERS_DIR"
  ( cd "$WORKER_SRC" && cargo build ) >"$REPORTS/build.log" 2>&1 || { echo "build failed:"; tail -20 "$REPORTS/build.log"; exit 1; }
  mkdir -p "$WORKERS_DIR"; cp "$WORKER_SRC/target/debug/iii-directory" "$WORKERS_DIR/iii-directory"
fi
[ -x "$WORKERS_DIR/iii-directory" ] || { echo "no iii-directory in $WORKERS_DIR (run without --no-build)"; exit 1; }

# effective engine config: substitute this dir's ABSOLUTE path into config.yaml
# (the engine doesn't guarantee the worker cwd, so skills_folder must be absolute).
sed "s|__E2E_DIR__|$ROOT_DIR|g" "$ROOT_DIR/config.yaml" > "$ENGINE_CONFIG"

# deterministic state: fresh global root + a known LOCAL override of `shell`
rm -rf "$GLOBAL"
# boot-time fixtures present BEFORE the engine starts, so the worker's startup
# log_fs_health scan exercises its skill-loaded / prompt-loaded / skipped-entry
# paths (an empty global root would scan to nothing). The uppercase namespace
# derives an invalid id and is skipped (SkipReason).
mkdir -p "$GLOBAL/bootns/prompts" "$GLOBAL/BadNS"
printf -- '---\ntype: index\ntitle: Boot Skill\n---\n# Boot\nPresent at worker boot.\n' > "$GLOBAL/bootns/index.md"
printf -- '---\ndescription: boot prompt\n---\nBoot prompt body\n'                  > "$GLOBAL/bootns/prompts/bootprompt.md"
printf -- '# bad\nuppercase namespace derives an invalid id; scan skips it.\n'      > "$GLOBAL/BadNS/index.md"
mkdir -p "$LOCAL/shell"
cat > "$LOCAL/shell/index.md" <<'MD'
---
title: shell (LOCAL override)
type: index
description: LOCAL override of the downloaded shell worker.
---
# shell (LOCAL OVERRIDE)
This local copy shadows the whole downloaded shell namespace.
MD

# alias fixtures (local) to exercise the SKILLS.md (plural) + SKILL.md (singular) aliases
mkdir -p "$LOCAL/aliasplural" "$LOCAL/aliassingular"
printf -- '---\ntitle: Plural Alias\ntype: index\n---\n# plural\n'   > "$LOCAL/aliasplural/SKILLS.md"
printf -- '---\ntitle: Singular Alias\ntype: index\n---\n# singular\n' > "$LOCAL/aliassingular/SKILL.md"

# fresh engine on $PORT (kill any test engine we previously left behind)
pkill -f "$ENGINE_CONFIG" 2>/dev/null || true
sleep 1
ENGINE_PID=""
teardown() {
  if [ "$KEEP" = 1 ]; then echo "    (--keep) engine left running pid=$ENGINE_PID on :$PORT"; return; fi
  [ -n "$ENGINE_PID" ] && kill "$ENGINE_PID" 2>/dev/null || true
  pkill -f "$WORKERS_DIR/iii-directory" 2>/dev/null || true
}
trap teardown EXIT INT TERM
echo "    start engine: $III --config $ENGINE_CONFIG  (port $PORT)"
"$III" --config "$ENGINE_CONFIG" >"$ENGINE_LOG" 2>&1 & ENGINE_PID=$!
REG=no; for _ in $(seq 1 60); do trig directory::skills::list --json '{}' >/dev/null 2>&1 && { REG=yes; break; }; sleep 0.5; done
[ "$REG" = yes ] || { echo "iii-directory did not register; engine log:"; tail -25 "$ENGINE_LOG"; exit 1; }
echo "    iii-directory registered on :$PORT"

# ── 0. boot reconcile: engine skill (iii) auto-downloaded on startup ─────────
# auto_download:true makes the boot-reconcile task pull the engine's OWN `iii`
# skill on startup — independent of worker::list (the engine is not a worker,
# so it never appears there) and BEFORE any explicit download below (§2).
# $GLOBAL was wiped at setup and seeds only bootns/BadNS, so iii/ appearing
# here can ONLY come from boot-reconcile.
echo "==> boot reconcile: engine skill auto-downloaded"
BR=no; for _ in $(seq 1 40); do [ -f "$GLOBAL/iii/SKILL.md" ] && { BR=yes; break; }; sleep 0.5; done
[ "$BR" = yes ] && ok "boot-reconcile auto-downloaded engine skill iii/ before any explicit download" || no "boot-reconcile auto-downloaded engine skill iii/"
[ -f "$GLOBAL/iii/.iii-skill-complete" ] && ok "engine-skill auto-download wrote the completion marker" || no "engine-skill auto-download wrote completion marker"
out=$(trig directory::skills::get id=iii/index)
jtrue "engine skill served via get id=iii/index (returns bare worker id)" "$out" '.id == "iii"'

# ── 1. registry HTTP proxy (live) ────────────────────────────────────────────
echo "==> registry proxy (live api.workers.iii.dev)"
out=$(trig directory::registry::workers::list --json '{}')
jtrue "registry::workers::list returns a non-empty workers array" "$out" '.workers | length > 0'
jtrue "registry::workers::list is cursor-paginated"               "$out" '.pagination.page_size > 0'
out=$(trig directory::registry::workers::info name=shell)
jtrue "registry::workers::info name=shell returns shell functions" "$out" '.api_reference.functions | map(.name) | any(startswith("shell::"))'

# ── 2. download (registry source) + on-disk structure / prefix-strip fix ─────
echo "==> download real workers + verify on-disk structure"
out=$(trig directory::skills::download worker=shell)
jtrue "download shell -> writes index.md"                "$out" '.skills_written | index("index.md") != null'
jtrue "download shell -> NO nested skills/ prefix"       "$out" '([.skills_written[] | startswith("skills/")] | any) | not'
out=$(trig directory::skills::download worker=iii)
jtrue "download iii  -> SKILL.md (singular) present"     "$out" '.skills_written | index("SKILL.md") != null'
jtrue "download iii  -> NOT nested under skills/SKILL.md" "$out" '.skills_written | index("skills/SKILL.md") == null'
trig directory::skills::download worker=database >/dev/null
trig directory::skills::download worker=coder    >/dev/null
[ -f "$GLOBAL/iii/SKILL.md" ]            && ok "on-disk: skills-home/iii/SKILL.md exists"        || no "on-disk: skills-home/iii/SKILL.md exists"
[ ! -e "$GLOBAL/iii/skills/SKILL.md" ]  && ok "on-disk: NO skills-home/iii/skills/SKILL.md"     || no "on-disk: NO skills-home/iii/skills/SKILL.md"
# the redundant-prefix bug would show as an IMMEDIATE <worker>/skills/ child (depth 2);
# legit deep namespaces like iii-directory/directory/skills/ (depth 3) are fine.
[ -z "$(find "$GLOBAL" -mindepth 2 -maxdepth 2 -type d -name skills 2>/dev/null)" ] && ok "on-disk: no redundant <worker>/skills/ prefix dirs" || no "on-disk: no redundant <worker>/skills/ prefix dirs"

# ── 3. skills reads ──────────────────────────────────────────────────────────
echo "==> skills reads"
out=$(trig directory::skills::list --json '{}')
jtrue "list includes downloaded shell/database/coder/iii overviews (bare ids)" "$out" '[.skills[].id] as $i | (["shell","database","coder","iii"] | all(. as $x | $i | index($x) != null))'
jtrue "list rows carry a non-empty description by default"        "$out" '[.skills[].description] | any(. != "" and . != null)'
out=$(trig directory::skills::list --json '{"include_description": false}')
jtrue "list include_description=false -> all descriptions empty"   "$out" '[.skills[].description] | all(. == "" or . == null)'
out=$(trig directory::skills::list --json '{"prefix": "database/"}')
# prefix filters on the raw on-disk id; the overview row displays as the bare
# `database`, so rows start with `database` (overview) or `database/` (sub-skills).
jtrue "list prefix=database/ -> only database rows"               "$out" '(.skills | length > 0) and ([.skills[].id] | all(startswith("database")))'

out=$(trig directory::skills::get id=shell/index)
has  "get shell/index -> LOCAL override body wins"               "$out" "LOCAL OVERRIDE"
out=$(trig directory::skills::get id=iii/index)
jtrue "get iii/index resolves (real SKILL.md alias), bare id"    "$out" '.id == "iii"'
has  "get iii/index -> real iii body content"                    "$out" "worker mesh"

# the `iii` worker ships skills/SKILL.md → flattened on disk to iii/SKILL.md and
# served under the `iii/index` id via the singular SKILL.md alias. Prove every
# form an agent might type for it returns that same real doc (headline case).
echo "==> iii: SKILL.md alias (every form of get id=iii)"
out=$(trig directory::skills::get id=iii)
jtrue "get id=iii (bare name) -> resolves, returns bare id"      "$out" '.id == "iii"'
jtrue "get id=iii -> title falls back to body H1 (\"iii\")"       "$out" '.title == "iii"'
jtrue "get id=iii -> type is null (SKILL.md omits frontmatter type)" "$out" '.type == null'
has   "get id=iii -> real SKILL.md body (\"worker mesh\")"        "$out" "worker mesh"
has   "get id=iii -> real SKILL.md body (registerWorker snippet)" "$out" "registerWorker"
iii_bare_body=$(printf '%s' "$out" | jq -r '.body')
out=$(trig directory::skills::get id=iii/SKILL.md)
jtrue "get id=iii/SKILL.md (explicit filename) -> bare iii"      "$out" '.id == "iii"'
out=$(trig directory::skills::get id=iii/index.md)
jtrue "get id=iii/index.md (.md suffix) -> bare iii"             "$out" '.id == "iii"'
out=$(trig directory::skills::get id=iii://iii)
jtrue "get id=iii://iii (URI + bare name) -> bare iii"           "$out" '.id == "iii"'
# every form converges on the identical real document body
iii_index_body=$(trig directory::skills::get id=iii/index | jq -r '.body')
if [ -n "$iii_bare_body" ] && [ "$iii_bare_body" = "$iii_index_body" ]; then
  ok "get id=iii body is identical to get id=iii/index body"
else
  no "get id=iii body is identical to get id=iii/index body"
fi
# the index id is served FROM SKILL.md — no generated iii/index.md exists on disk
[ ! -e "$GLOBAL/iii/index.md" ] && ok "on-disk: iii/index served from SKILL.md (no iii/index.md file)" || no "on-disk: iii/index served from SKILL.md (no iii/index.md file)"
# the SKILL.md frontmatter description surfaces in list rows (get output has none)
out=$(trig directory::skills::list --json '{}')
jtrue "list: iii row carries the SKILL.md frontmatter description" "$out" '([.skills[]|select(.id=="iii")|.description][0] // "") | contains("WebSocket-routed worker mesh")'

out=$(trig directory::skills::get id=database/iii-database/query)
jtrue "get database/iii-database/query -> real deep skill title" "$out" '.title == "Run a read-only SQL query and return rows"'
out=$(trig directory::skills::get id=database/query)
iserr "get database/query (miss) -> error envelope"              "$out"
has   "miss -> D110 code"                                        "$out" "D110"
has   "miss -> not_found type"                                   "$out" "not_found"
has   "miss -> deep skill suggested (database/iii-database/query)" "$out" "database/iii-database/query"
hasnt "miss -> overview suggestion is bare (no database/index)"  "$out" "database/index"
has   "miss -> next action (directory::skills::list)"            "$out" "directory::skills::list"
hasnt "miss -> clean prose, NO escaped-JSON fix envelope"        "$out" '"fix"'

out=$(trig directory::skills::index --json '{}')
jtrue "index returns a body + worker count"                      "$out" '(.workers_count >= 1) and (.body | length > 0)'
has   "index includes dive-deeper URLs"                          "$out" "Dive deeper: https://workers.iii.dev/workers/"
has   "index reflects the LOCAL shell override title"            "$out" "shell (LOCAL override)"

# ── 4. prompts (the real workers ship none, so lay down our own fixtures) ────
echo "==> prompts"
# a valid prompt (frontmatter `description`) + one WITHOUT (must be silently skipped)
mkdir -p "$GLOBAL/promptns/prompts"
printf -- '---\ndescription: A test greeting prompt.\n---\nHello {{name}}!\n' > "$GLOBAL/promptns/prompts/greeting.md"
printf -- 'Hello, but this prompt has no frontmatter description.\n'          > "$GLOBAL/promptns/prompts/nodesc.md"
out=$(trig directory::prompts::list --json '{}')
jtrue "prompts::list returns a prompts array"                    "$out" '.prompts | type == "array"'
jtrue "prompts::list includes the described fixture prompt"      "$out" '[.prompts[].name] | index("greeting") != null'
jtrue "prompts::list SKIPS the no-description prompt"            "$out" '[.prompts[].name] | index("nodesc") == null'
out=$(trig directory::prompts::get name=greeting)
jtrue "prompts::get greeting -> name + body + description"       "$out" '.name == "greeting" and (.body | contains("Hello")) and (.description == "A test greeting prompt.")'
out=$(trig directory::prompts::get --json '{"name":"nodesc"}')
iserr "prompts::get nodesc (silently skipped) -> not_found"     "$out"
has   "  └ D210 not_found"                                      "$out" "D210"
# local prompt override (prompts now honour local_skills_folder like skills):
# a LOCAL namespace shadows the same-named global namespace's prompts.
mkdir -p "$GLOBAL/overridens/prompts" "$LOCAL/overridens/prompts"
printf -- '---\ndescription: GLOBAL prompt (shadowed)\n---\nglobal body\n' > "$GLOBAL/overridens/prompts/g.md"
printf -- '---\ndescription: LOCAL override prompt\n---\nlocal body\n'      > "$LOCAL/overridens/prompts/l.md"
out=$(trig directory::prompts::list --json '{}')
jtrue "prompts::list includes the LOCAL override prompt"        "$out" '[.prompts[].name] | index("l") != null'
jtrue "prompts::list shadows the global prompt in an overridden ns" "$out" '[.prompts[].name] | index("g") == null'
out=$(trig directory::prompts::get name=l)
jtrue "prompts::get l -> LOCAL override body"                   "$out" '(.body | contains("local body")) and (.description == "LOCAL override prompt")'
out=$(trig directory::prompts::get --json '{"name":"g"}')
iserr "prompts::get g (shadowed global) -> not_found"          "$out"

# ── 5. engine introspection proxy (re-added WITHOUT how_guide) ──────────────
echo "==> engine::functions::info (no how_guide)"
out=$(trig directory::engine::functions::info function_id=directory::skills::list)
jtrue "functions::info returns the requested function"           "$out" '.function_id == "directory::skills::list"'
jtrue "functions::info carries a request schema"                 "$out" '.request_schema != null'
hasnt "functions::info has NO how_guide field"                   "$out" "how_guide"

# ── 6. security: git source rejects dangerous repo URLs ─────────────────────
echo "==> security: git repo URL validation (RCE guard)"
out=$(trig directory::skills::download --json '{"repo":"ext::sh -c id","skill":"x"}')
iserr "download repo=ext::… is rejected (no command execution)"  "$out"
out=$(trig directory::skills::download --json '{"repo":"http://insecure/x.git","skill":"x"}')
iserr "download repo=http:// (non-https) is rejected"            "$out"

# ── 7. EDGE CASES / adversarial (try to break it) ───────────────────────────
echo "==> edge cases / adversarial"

# get: path traversal must be rejected AND must not leak filesystem content
out=$(trig directory::skills::get --json '{"id":"../../../../etc/passwd"}')
iserr "get id=../../../../etc/passwd -> rejected"                "$out"
hasnt "  └ traversal did NOT leak /etc/passwd (no 'root:')"      "$out" "root:"
out=$(trig directory::skills::get --json '{"id":"shell/../../../etc/passwd"}')
iserr "get id=shell/../../../etc/passwd -> rejected"             "$out"
hasnt "  └ nested traversal did NOT leak"                        "$out" "root:"
# get: empty / whitespace id
out=$(trig directory::skills::get --json '{"id":""}')
iserr "get id=\"\" -> rejected"                                  "$out"
# get: uppercase id (segments are lowercase-only)
out=$(trig directory::skills::get --json '{"id":"DATABASE/INDEX"}')
iserr "get id=DATABASE/INDEX (uppercase) -> rejected"            "$out"
# get: non-ASCII id
out=$(trig directory::skills::get --json '{"id":"shell/индекс"}')
iserr "get id with non-ASCII segment -> rejected"               "$out"
# get: absolute path id
out=$(trig directory::skills::get --json '{"id":"/etc/passwd"}')
iserr "get id=/etc/passwd (absolute) -> rejected"               "$out"
# get: iii:// URI form resolves (returns bare id)
out=$(trig directory::skills::get --json '{"id":"iii://database/index"}')
jtrue "get id=iii://database/index resolves to bare database"   "$out" '.id == "database"'
# get: trailing .md suffix resolves (returns bare id)
out=$(trig directory::skills::get --json '{"id":"database/index.md"}')
jtrue "get id=database/index.md (suffix) -> bare database"      "$out" '.id == "database"'
# get: bare worker name -> the overview, returned as the bare id
out=$(trig directory::skills::get id=database)
jtrue "get id=database (bare) -> bare database"                 "$out" '.id == "database"'
# alias coverage via local fixtures
out=$(trig directory::skills::get id=aliasplural/index)
jtrue "SKILLS.md (plural) alias -> aliasplural/index"           "$out" '.title == "Plural Alias"'
out=$(trig directory::skills::get id=aliassingular/index)
jtrue "SKILL.md (singular) alias -> aliassingular/index"        "$out" '.title == "Singular Alias"'
# whole-namespace override shadows siblings: shell/exec EXISTS in the downloaded
# global shell, but the LOCAL shell namespace (index.md only) shadows ALL shell/*
out=$(trig directory::skills::get id=shell/exec)
iserr "get shell/exec -> NOT FOUND (whole-namespace override shadows it)" "$out"
# a non-overridden worker's deep skill is still reachable
out=$(trig directory::skills::get id=database/iii-database/execute)
jtrue "get database/iii-database/execute (no override) -> visible" "$out" '.id == "database/iii-database/execute"'

# list filters: bogus type / no-match search -> empty, not an error
out=$(trig directory::skills::list --json '{"type":"zzz-nope"}')
jtrue "list type=zzz-nope -> empty (no error)"                  "$out" '.skills | length == 0'
out=$(trig directory::skills::list --json '{"search":"zzzz-nomatch-zzzz"}')
jtrue "list search=no-match -> empty (no error)"               "$out" '.skills | length == 0'

# download argument validation
out=$(trig directory::skills::download --json '{}')
iserr "download {} (neither repo nor worker) -> rejected"       "$out"
out=$(trig directory::skills::download --json '{"repo":"https://github.com/x/y","skill":"z","worker":"w"}')
iserr "download repo+worker (both) -> rejected"                 "$out"
out=$(trig directory::skills::download --json '{"repo":"https://github.com/x/y"}')
iserr "download repo without skill -> rejected"                 "$out"
# security: worker-name traversal + dangerous git URLs
out=$(trig directory::skills::download --json '{"worker":"../../etc"}')
iserr "download worker=../../etc (traversal) -> rejected"       "$out"
out=$(trig directory::skills::download --json '{"repo":"file:///etc/passwd","skill":"x"}')
iserr "download repo=file:// -> rejected"                       "$out"
out=$(trig directory::skills::download --json '{"repo":"--upload-pack=/tmp/x","skill":"y"}')
iserr "download repo=--upload-pack (arg injection) -> rejected" "$out"
out=$(trig directory::skills::download --json '{"repo":"git::ext::sh -c id","skill":"x"}')
iserr "download repo with '::' transport -> rejected"          "$out"
# download a worker that doesn't exist -> friendly D310, NO internal URL leak
out=$(trig directory::skills::download worker=zzz-nonexistent-worker-zzz)
iserr "download nonexistent worker -> error"                    "$out"
hasnt "  └ download miss: no internal registry URL leak"        "$out" "api.workers.iii.dev"

# prompts::get miss -> friendly D210 not_found + next action (was a bare string)
out=$(trig directory::prompts::get --json '{"name":"does-not-exist"}')
iserr "prompts::get nonexistent -> error"                       "$out"
has   "  └ D210 not_found"                                      "$out" "D210"
has   "  └ next action (directory::prompts::list)"              "$out" "directory::prompts::list"

# engine::functions::info for a nonexistent function id
out=$(trig directory::engine::functions::info function_id=zzz::nope::nonexistent)
iserr "engine::functions::info nonexistent fn -> error"         "$out"

# registry::workers::info miss -> friendly D310, NO internal URL / HTTP status leak
out=$(trig directory::registry::workers::info name=zzz-nonexistent-worker-zzz)
iserr "registry::workers::info nonexistent -> error"            "$out"
has   "  └ D310 not_found"                                      "$out" "D310"
has   "  └ next action (registry::workers::list)"               "$out" "directory::registry::workers::list"
hasnt "  └ no internal registry URL leak"                       "$out" "api.workers.iii.dev"
hasnt "  └ no raw HTTP status leak"                             "$out" "HTTP 404"

# ── 8. explicit, intent-named download functions (schema self-validates source) ──
echo "==> explicit download_from_registry / download_from_repo"
out=$(trig directory::skills::download_from_registry worker=coder)
jtrue "download_from_registry worker=coder -> writes skills"     "$out" '.skills_written | length > 0'
jtrue "download_from_registry -> source.kind == registry"        "$out" '.source.kind == "registry"'
out=$(trig directory::skills::download_from_registry worker=zzz-nope-zzz)
iserr "download_from_registry nonexistent -> error"             "$out"
has   "  └ D310 not_found"                                      "$out" "D310"
hasnt "  └ no internal registry URL leak"                       "$out" "api.workers.iii.dev"
out=$(trig directory::skills::download_from_repo --json '{"repo":"ext::sh -c id","skill":"x"}')
iserr "download_from_repo repo=ext:: -> rejected (RCE guard holds)" "$out"

# ── 9. adversarial: break the recent refactor (errors / split / name validation) ──
echo "==> adversarial: break the recent refactor"

# registry::workers::info name flows into the /w/{name} URL path — a crafted
# name must NOT traverse out of /w/ or inject a query/fragment on the host.
out=$(trig directory::registry::workers::info name=../../admin)
iserr "registry::info name=../../admin (path traversal) -> rejected" "$out"
has   "  └ D311 invalid_input"                                   "$out" "D311"
hasnt "  └ no internal registry URL leak"                        "$out" "api.workers.iii.dev"
out=$(trig directory::registry::workers::info --json '{"name":"shell/../../etc"}')
iserr "registry::info name=shell/../../etc -> rejected"          "$out"
out=$(trig directory::registry::workers::info --json '{"name":"x?admin=1"}')
iserr "registry::info name=x?admin=1 (query injection) -> rejected" "$out"
out=$(trig directory::registry::workers::info --json '{"name":"x#frag"}')
iserr "registry::info name=x#frag (fragment injection) -> rejected" "$out"
out=$(trig directory::registry::workers::info --json '{"name":"SHELL"}')
iserr "registry::info name=SHELL (uppercase) -> rejected"        "$out"
out=$(trig directory::registry::workers::info --json '{"name":"工具"}')
iserr "registry::info name=non-ASCII -> rejected"               "$out"
out=$(trig directory::registry::workers::info --json '{"name":""}')
iserr "registry::info name=\"\" (empty) -> rejected"             "$out"
# real hyphenated worker name still accepted (the fix must not over-reject)
out=$(trig directory::registry::workers::info name=shell)
jtrue "registry::info name=shell (real, hyphen-safe) -> ok"      "$out" '.api_reference.functions | length > 0'
# version + tag together / nonexistent version -> clean error, no URL leak
out=$(trig directory::registry::workers::info --json '{"name":"shell","version":"1.0.0","tag":"latest"}')
iserr "registry::info version+tag (both) -> rejected"            "$out"
out=$(trig directory::registry::workers::info --json '{"name":"shell","version":"99.99.99"}')
iserr "registry::info shell@99.99.99 (no such version) -> error" "$out"
hasnt "  └ no internal registry URL leak"                        "$out" "api.workers.iii.dev"

# download_from_registry (NEW fn) — required worker + validation + no leak
out=$(trig directory::skills::download_from_registry --json '{}')
iserr "download_from_registry {} (missing required worker) -> rejected" "$out"
out=$(trig directory::skills::download_from_registry --json '{"worker":"../../etc"}')
iserr "download_from_registry worker=../../etc (traversal) -> rejected" "$out"
hasnt "  └ no internal registry URL leak"                        "$out" "api.workers.iii.dev"
out=$(trig directory::skills::download_from_registry --json '{"worker":"SHELL"}')
iserr "download_from_registry worker=SHELL (uppercase) -> rejected" "$out"
out=$(trig directory::skills::download_from_registry --json '{"worker":"shell","version":"1.0.0","tag":"latest"}')
iserr "download_from_registry version+tag (both) -> rejected"    "$out"
out=$(trig directory::skills::download_from_registry --json '{"worker":"   "}')
iserr "download_from_registry worker=whitespace -> rejected"     "$out"
# idempotent re-download (overwrite) must stay healthy
out=$(trig directory::skills::download_from_registry worker=coder)
jtrue "download_from_registry coder (repeat) -> still ok"        "$out" '.skills_written | length > 0'

# download_from_repo (NEW fn) — required repo+skill + traversal/RCE guards
out=$(trig directory::skills::download_from_repo --json '{"repo":"https://github.com/x/y"}')
iserr "download_from_repo missing required skill -> rejected"    "$out"
out=$(trig directory::skills::download_from_repo --json '{"repo":"https://github.com/x/y","skill":"../../../etc"}')
iserr "download_from_repo skill=../../../etc (dest traversal) -> rejected" "$out"
out=$(trig directory::skills::download_from_repo --json '{"repo":"file:///etc/passwd","skill":"x"}')
iserr "download_from_repo repo=file:// -> rejected"              "$out"
out=$(trig directory::skills::download_from_repo --json '{"repo":"--upload-pack=/tmp/x","skill":"y"}')
iserr "download_from_repo repo=--upload-pack (arg injection) -> rejected" "$out"

# prose not_found_message robustness — a long-but-VALID missing id must NOT panic
longid="$(printf 'aa/%.0s' $(seq 1 150))index"
out=$(trig directory::skills::get --json "{\"id\":\"$longid\"}")
iserr "get very-long (~455 char) valid id (miss) -> clean error" "$out"
has   "  └ still D110 not_found (no panic on long id)"           "$out" "D110"
# trailing slash / empty segment ids -> rejected
out=$(trig directory::skills::get --json '{"id":"database/"}')
iserr "get id=database/ (trailing slash) -> rejected"            "$out"
out=$(trig directory::skills::get --json '{"id":"database//query"}')
iserr "get id=database//query (empty segment) -> rejected"       "$out"
# a miss whose id collides with the prose format itself must not break parsing
out=$(trig directory::skills::get --json '{"id":"not_found/d110"}')
iserr "get id=not_found/d110 (prose-lookalike miss) -> clean error" "$out"

# ── 10. dumb-LLM scenarios: every realistic mistake a confused agent makes ────
echo "==> dumb-LLM scenarios"

# (a) wrong / hallucinated FUNCTION names -> engine routing error
out=$(trig directory::skills::read id=database/index)
iserr "dumb: wrong verb 'skills::read' -> error"               "$out"
out=$(trig directory::skill::get id=database/index)
iserr "dumb: singular 'skill::get' -> error"                   "$out"
out=$(trig skills::get id=database/index)
iserr "dumb: forgot 'directory::' prefix -> error"             "$out"
out=$(trig directory::skills::download_skill worker=shell)
iserr "dumb: made-up 'download_skill' -> error"                "$out"

# (b) id vs function_id confusion -> targeted D112 hint (not a raw segment error)
out=$(trig directory::skills::get id=database::execute)
iserr "dumb: passed function_id to get -> rejected"            "$out"
has   "  └ D112 (that's a function id, not a skill id)"        "$out" "D112"
has   "  └ hint names the FUNCTION id confusion"              "$out" "FUNCTION id"
has   "  └ recovery points at directory::skills::list"        "$out" "directory::skills::list"
out=$(trig directory::skills::get id=shell::fs::mv)
iserr "dumb: passed 'shell::fs::mv' function id to get -> D112" "$out"
has   "  └ D112"                                               "$out" "D112"

# (c) hallucinated skill ids -> D110 recovery with did-you-mean
out=$(trig directory::skills::get id=database/run-query)
iserr "dumb: hallucinated 'database/run-query' -> error"       "$out"
has   "  └ D110 + did you mean"                                "$out" "Did you mean"
out=$(trig directory::skills::get id=database/execute)
iserr "dumb: 'database/execute' (dropped the nesting) -> error" "$out"
has   "  └ suggests the real nested id"                        "$out" "database/iii-database/execute"

# (d) wrong PARAMETER names (serde: required field missing / unknown ignored)
out=$(trig directory::skills::get --json '{"skill_id":"database/index"}')
iserr "dumb: wrong param 'skill_id' (id missing) -> error"     "$out"
out=$(trig directory::skills::get --json '{"name":"database/index"}')
iserr "dumb: wrong param 'name' for get -> error"              "$out"
out=$(trig directory::skills::download_from_registry --json '{"name":"shell"}')
iserr "dumb: used 'name' instead of 'worker' -> error"         "$out"

# (e) TYPE confusion (number / array / null where a string is wanted)
out=$(trig directory::skills::get --json '{"id":123}')
iserr "dumb: id as a number -> error"                          "$out"
out=$(trig directory::skills::get --json '{"id":["database/index"]}')
iserr "dumb: id as an array -> error"                          "$out"
out=$(trig directory::skills::get --json '{"id":null}')
iserr "dumb: id as null -> error"                              "$out"
out=$(trig directory::skills::list --json '{"search":123}')
iserr "dumb: search as a number -> error"                      "$out"

# (f) copy-paste from prior output — the ergonomic aliases should ABSORB these
out=$(trig directory::skills::get --json '{"id":"https://workers.iii.dev/workers/database?tab=api"}')
iserr "dumb: pasted the dive-deeper URL as id -> rejected"     "$out"
out=$(trig directory::skills::get id=database/index.md)
jtrue "dumb: pasted 'database/index.md' link -> RESOLVES (bare)" "$out" '.id == "database"'
out=$(trig directory::skills::get id=iii://database/index)
jtrue "dumb: pasted legacy 'iii://database/index' -> RESOLVES (bare)" "$out" '.id == "database"'
out=$(trig directory::skills::get id=database)
jtrue "dumb: typed bare 'database' -> RESOLVES"                "$out" '.id == "database"'

# (g) natural-language ids -> rejected (spaces / punctuation)
out=$(trig directory::skills::get --json '{"id":"the database query skill"}')
iserr "dumb: natural-language id (spaces) -> rejected"         "$out"
out=$(trig directory::skills::get --json '{"id":"How do I run SQL?"}')
iserr "dumb: a question as the id -> rejected"                 "$out"

# (h) prompts vs skills confusion
out=$(trig directory::prompts::get name=shell)
iserr "dumb: asked a SKILL ('shell') via prompts::get -> error" "$out"
has   "  └ D210 not_found"                                     "$out" "D210"
out=$(trig directory::prompts::get --json '{"name":"database/index"}')
iserr "dumb: skill id as a prompt name (has '/') -> rejected"  "$out"
out=$(trig directory::prompts::get --json '{"prompt":"x"}')
iserr "dumb: wrong param 'prompt' (name missing) -> error"     "$out"

# (i) download confusion
out=$(trig directory::skills::download --json '{"worker":"shell","skill":"x"}')
iserr "dumb: mixed registry+repo fields on the alias -> rejected" "$out"
out=$(trig directory::skills::download_from_repo --json '{"worker":"shell"}')
iserr "dumb: repo fn with 'worker' (repo/skill missing) -> error" "$out"
out=$(trig directory::skills::download_from_registry --json '{"worker":"shell","tag":"stable"}')
iserr "dumb: made-up tag 'stable' -> error (no such tag)"      "$out"
hasnt "  └ no internal registry URL leak"                      "$out" "api.workers.iii.dev"

# (j) junk args to no-arg / filterable reads -> ignored, still works (no crash)
out=$(trig directory::skills::index --json '{"worker":"database"}')
jtrue "dumb: arg to no-arg index -> ignored, still renders"    "$out" '.body | length > 0'
out=$(trig directory::skills::list --json '{"filter":"database"}')
jtrue "dumb: unknown 'filter' param -> ignored, returns list"  "$out" '.skills | length > 0'

# (k) engine::functions::info confusion
out=$(trig directory::engine::functions::info function_id=shell)
iserr "dumb: bare worker name to functions::info -> error"     "$out"
out=$(trig directory::engine::functions::info --json '{"function":"directory::skills::get"}')
iserr "dumb: wrong param 'function' (function_id missing) -> error" "$out"

# ── 11. auto-download + boot reconcile (config has auto_download: true) ───────
# main.rs registers the internal directory::__on_worker_added handler and spawns
# the boot-reconcile task only when auto_download is on. The handler is what an
# engine `worker` add event invokes; we drive it directly here (no daemon).
echo "==> auto-download + boot reconcile (auto_download:true)"
out=$(trig directory::engine::functions::info function_id=directory::__on_worker_added)
jtrue "auto-download handler registered (auto_download:true)"    "$out" '.function_id == "directory::__on_worker_added"'
# happy path: a worker-add event downloads that worker's skills + invalidates cache
rm -rf "$GLOBAL/coder"
[ ! -e "$GLOBAL/coder" ] && ok "precondition: coder/ absent before auto-download" || no "precondition: coder/ absent"
out=$(trig directory::__on_worker_added --json '{"worker":"coder"}')
jtrue "__on_worker_added {worker:coder} -> ok"                   "$out" '.ok == true'
[ -d "$GLOBAL/coder" ] && ok "auto-download wrote coder/ to skills-home" || no "auto-download wrote coder/ to skills-home"
[ -f "$GLOBAL/coder/.iii-skill-complete" ] && ok "auto-download wrote the completion marker" || no "auto-download wrote the completion marker"
out=$(trig directory::skills::list --json '{"prefix":"coder/"}')
jtrue "auto-downloaded coder visible in list (cache invalidated)" "$out" '.skills | length > 0'
# missing 'worker' field -> handler skips gracefully (still ok)
out=$(trig directory::__on_worker_added --json '{}')
jtrue "__on_worker_added {} (no worker field) -> ok (skip)"      "$out" '.ok == true'
# nonexistent worker -> registry 404 is benign (still ok, no crash)
out=$(trig directory::__on_worker_added --json '{"worker":"zzz-nonexistent-worker-zzz"}')
jtrue "__on_worker_added nonexistent -> ok (404 benign)"         "$out" '.ok == true'
# invalid worker name -> validated inside download_worker_skills, swallowed (still ok)
out=$(trig directory::__on_worker_added --json '{"worker":"../../etc"}')
jtrue "__on_worker_added invalid name -> ok (rejected internally)" "$out" '.ok == true'
# re-add an already-present worker (idempotent overwrite path). The in-flight
# dedup guard's concurrent-claim branch is covered by the download.rs unit test
# `in_flight_concurrent_claim_blocked` (firing real concurrent downloads here
# overloads the single worker and is not a meaningful black-box assertion).
out=$(trig directory::__on_worker_added --json '{"worker":"database"}')
jtrue "__on_worker_added repeat (already present) -> ok"         "$out" '.ok == true'

# ── 12. git source: a REAL clone — covers sources/git.rs (run_git_clone + copy) ──
# Network-dependent (clones a public repo). Set SKIP_GIT=1 to skip in offline CI.
if [ "${SKIP_GIT:-0}" != "1" ]; then
  echo "==> git source (real clone of github.com/anthropics/skills)"
  out=$(TRIG_TIMEOUT=120 trig directory::skills::download_from_repo --json '{"repo":"https://github.com/anthropics/skills","skill":"mcp-builder"}')
  jtrue "download_from_repo (real git clone) -> writes skills"   "$out" '.skills_written | length > 0'
  jtrue "download_from_repo -> source.kind == repo"             "$out" '.source.kind == "repo"'
  [ -f "$GLOBAL/mcp-builder/SKILL.md" ] && ok "git clone wrote mcp-builder/SKILL.md" || no "git clone wrote mcp-builder/SKILL.md"
  out=$(trig directory::skills::get id=mcp-builder)
  jtrue "get mcp-builder (cloned, SKILL.md alias) -> bare id"   "$out" '.id == "mcp-builder"'
  # clone succeeds but the skill folder is absent in the repo -> copy-step error
  out=$(TRIG_TIMEOUT=120 trig directory::skills::download_from_repo --json '{"repo":"https://github.com/anthropics/skills","skill":"zzz-nonexistent-skill"}')
  iserr "download_from_repo skill absent in repo -> error"      "$out"
  # bad branch -> the git clone itself fails (non-zero exit)
  out=$(TRIG_TIMEOUT=120 trig directory::skills::download_from_repo --json '{"repo":"https://github.com/anthropics/skills","skill":"mcp-builder","branch":"zzz-no-such-branch"}')
  iserr "download_from_repo bad branch -> clone fails"          "$out"
else
  echo "==> git source: SKIPPED (SKIP_GIT=1)"
fi

# ── 13. fault injection (deterministic) — covers fs_source / read error arms ──
echo "==> fault injection (oversized / empty / duplicate-id / unreadable)"
# oversized body (> SKILL_BODY_MAX_BYTES = 256 KiB) -> read rejects on get
mkdir -p "$GLOBAL/oversize"
{ printf -- '---\ntype: index\n---\n# Big\n'; head -c 300000 /dev/zero | tr '\0' 'x'; } > "$GLOBAL/oversize/index.md"
out=$(trig directory::skills::get id=oversize/index)
iserr "get oversize/index (> 256 KiB body cap) -> rejected"     "$out"
# frontmatter-only (empty body) -> read rejects
mkdir -p "$GLOBAL/emptyskill"
printf -- '---\ntype: index\n---\n' > "$GLOBAL/emptyskill/index.md"
out=$(trig directory::skills::get id=emptyskill/index)
iserr "get emptyskill/index (empty body) -> rejected"           "$out"
# duplicate id (index.md + SKILLS.md both derive <ns>/index) -> scan skips one
mkdir -p "$GLOBAL/dupid"
printf -- '---\ntype: index\n---\n# A\nbody a\n' > "$GLOBAL/dupid/index.md"
printf -- '---\ntype: index\n---\n# B\nbody b\n' > "$GLOBAL/dupid/SKILLS.md"
# unreadable prompt (perm 000) -> scan_prompts records a read SkipReason
mkdir -p "$GLOBAL/permns/prompts"
printf -- '---\ndescription: x\n---\nbody\n' > "$GLOBAL/permns/prompts/p.md"; chmod 000 "$GLOBAL/permns/prompts/p.md"
# list + index + prompts::list force a full scan over all of the above (skip arms)
out=$(trig directory::skills::list --json '{}')
jtrue "skills::list healthy with fault fixtures (scan skips the bad ones)" "$out" '.skills | length > 0'
out=$(trig directory::skills::index --json '{}')
jtrue "skills::index still renders with fault fixtures present"  "$out" '.body | length > 0'
out=$(trig directory::prompts::list --json '{}')
jtrue "prompts::list healthy with an unreadable prompt present"  "$out" '.prompts | type == "array"'
chmod 644 "$GLOBAL/permns/prompts/p.md" 2>/dev/null || true     # restore so teardown can clean
# unreadable skill file (perm 000) -> the body read fails on get (read-error arm)
mkdir -p "$GLOBAL/permskill"
printf -- '---\ntype: index\n---\n# P\nbody\n' > "$GLOBAL/permskill/index.md"; chmod 000 "$GLOBAL/permskill/index.md"
out=$(trig directory::skills::get id=permskill/index)
iserr "get permskill/index (unreadable file, perm 000) -> rejected" "$out"
chmod 644 "$GLOBAL/permskill/index.md" 2>/dev/null || true

# worker still healthy after all that abuse
out=$(trig directory::skills::list --json '{}')
jtrue "worker still healthy after adversarial inputs"           "$out" '.skills | length > 0'

# ── summary ──────────────────────────────────────────────────────────────────
echo
echo "════════════════════════════════════════════"
echo "  E2E RESULT: $PASS passed, $FAIL failed"
if [ "$FAIL" -eq 0 ]; then
  echo "  ✅ ALL PASSED"
  echo "════════════════════════════════════════════"
  exit 0
else
  echo "  ❌ FAILURES:"; printf '    - %s\n' "${FAILED[@]}"
  echo "════════════════════════════════════════════"
  exit 1
fi
