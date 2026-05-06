# Slim Harness Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Trim the `harness` meta-worker from a 22-worker bundle to a 14-worker bundle and delete 28 unused crates from the repo, leaving the chat surface working with Anthropic and OpenAI providers plus shell tools.

**Architecture:** Edit four files inside `harness/` (lib.rs, iii.worker.yaml, demo.sh, ARCHITECTURE.md), recreate the missing yaml manifest, then `git rm -r` 28 crate directories that have no remaining consumer. No source-level edits to any kept worker — `auth-credentials` stays so providers' existing `auth::get_token` registration path keeps working.

**Tech Stack:** Rust 2021, `cargo`, iii-sdk 0.11.3, Vite + React 18 (UI not modified).

**Spec:** `docs/superpowers/specs/2026-05-06-slim-harness-design.md`

**Working directory note:** All paths are relative to the repo root `/Users/ytallolayon/workspaces/personal/motia/workers/` unless otherwise stated. Run all `cargo` and `git` commands from there.

---

## Task 1: Recreate `harness/iii.worker.yaml`

**Why first:** The integration test at `harness/tests/integration.rs:7` reads this file via `include_str!`. Right now the file is missing, so the test cannot pass. Creating it before changing `lib.rs` means each subsequent task lands the repo in a buildable+testable state.

**Files:**
- Create: `harness/iii.worker.yaml`

**Reference for schema:** `turn-orchestrator/iii.worker.yaml` (has a `dependencies:` block we mirror).

- [ ] **Step 1: Create `harness/iii.worker.yaml` with the 14-worker dependency list**

```yaml
iii: v1
name: harness
language: rust
deploy: binary
manifest: Cargo.toml
bin: iii-harness
description: Meta-worker that composes the modular workers backing the iii chat surface.
dependencies:
  turn-orchestrator: "^0.1.0"
  provider-router: "^0.1.0"
  session-tree: "^0.1.0"
  session-inbox: "^0.1.0"
  models-catalog: "^0.1.0"
  hook-fanout: "^0.1.0"
  policy-denylist: "^0.1.0"
  shell-bash: "^0.1.0"
  shell-filesystem: "^0.1.0"
  subagent: "^0.1.0"
  provider-anthropic: "^0.1.0"
  provider-openai: "^0.1.0"
  auth-credentials: "^0.1.0"
  llm-budget: "^0.1.0"
```

- [ ] **Step 2: Verify the integration test now references a present file**

Run: `cd harness && cargo build --tests 2>&1 | tail -5`
Expected: build succeeds (no `include_str!` error). It is fine if the test itself fails at this point — we will fix that in Task 2.

- [ ] **Step 3: Commit**

```bash
git add harness/iii.worker.yaml
git -c commit.gpgsign=false commit -m "feat(harness): add iii.worker.yaml manifest with slim dependency list"
```

---

## Task 2: Replace `EXPECTED_WORKERS` in `harness/src/lib.rs`

**Files:**
- Modify: `harness/src/lib.rs:18-41`

**Existing code at `harness/src/lib.rs:18`:**

```rust
pub const EXPECTED_WORKERS: &[&str] = &[
    "turn-orchestrator",
    "provider-router",
    "context-compaction",
    "session-tree",
    "session-corpus",
    "document-extract",
    "models-catalog",
    "auth-credentials",
    "auth-rbac",
    "audit-log",
    "policy-denylist",
    "dlp-scrubber",
    "guardrails",
    "llm-budget",
    "session-inbox",
    "hook-fanout",
    "shell-bash",
    "shell-filesystem",
    "subagent",
    "provider-cli",
    "provider-anthropic",
    "provider-openai",
];
```

- [ ] **Step 1: Run the existing integration test first to see it fail (TDD anchor)**

Run: `cd harness && cargo test -p iii-harness expected_workers_matches_yaml_dependency_count 2>&1 | tail -10`
Expected: FAIL with assertion `22 != 14` (or similar — `EXPECTED_WORKERS.len()` is 22, yaml has 14).

- [ ] **Step 2: Replace the array with the 14-worker slim list**

Replace lines 18-41 of `harness/src/lib.rs` with:

```rust
pub const EXPECTED_WORKERS: &[&str] = &[
    "turn-orchestrator",
    "provider-router",
    "session-tree",
    "session-inbox",
    "models-catalog",
    "hook-fanout",
    "policy-denylist",
    "shell-bash",
    "shell-filesystem",
    "subagent",
    "provider-anthropic",
    "provider-openai",
    "auth-credentials",
    "llm-budget",
];
```

- [ ] **Step 3: Run the integration test to verify it now passes**

Run: `cd harness && cargo test -p iii-harness expected_workers_matches_yaml_dependency_count 2>&1 | tail -10`
Expected: PASS.

- [ ] **Step 4: Run all harness tests to confirm nothing else broke**

Run: `cd harness && cargo test -p iii-harness 2>&1 | tail -15`
Expected: all tests pass, including `library_exports_register_entry_point` and `expected_workers_is_unique_and_non_empty`.

- [ ] **Step 5: Commit**

```bash
git add harness/src/lib.rs
git -c commit.gpgsign=false commit -m "refactor(harness): slim EXPECTED_WORKERS to 14 workers"
```

---

## Task 3: Update `harness/scripts/demo.sh`

**Files:**
- Modify: `harness/scripts/demo.sh:32-62` (the `WORKERS=( ... )` array and the `ensure_auth_secret` function)

**Existing code at `harness/scripts/demo.sh:32`:**

```bash
WORKERS=(
  turn-orchestrator provider-router context-compaction
  session-tree session-corpus document-extract models-catalog
  auth-credentials auth-rbac audit-log policy-denylist
  dlp-scrubber guardrails llm-budget
  session-inbox hook-fanout
  shell-bash shell-filesystem subagent
  provider-cli provider-anthropic provider-openai
)
```

- [ ] **Step 1: Replace the `WORKERS` array with the slim list**

Replace lines 32-40 of `harness/scripts/demo.sh` with:

```bash
WORKERS=(
  turn-orchestrator provider-router
  session-tree session-inbox
  models-catalog hook-fanout policy-denylist
  shell-bash shell-filesystem subagent
  provider-anthropic provider-openai
  auth-credentials llm-budget
)
```

- [ ] **Step 2: Remove the `ensure_auth_secret` function and its caller**

Two edits:

**Edit A —** in `harness/scripts/demo.sh:42-46`, replace:

```bash
ensure_dirs() {
  mkdir -p "$DEMO_DIR/pids" "$DEMO_DIR/logs"
  ensure_auth_secret
}
```

with:

```bash
ensure_dirs() {
  mkdir -p "$DEMO_DIR/pids" "$DEMO_DIR/logs"
}
```

**Edit B —** in `harness/scripts/demo.sh:48-62`, delete the entire `ensure_auth_secret` function:

```bash
# auth-rbac refuses to start without AUTH_HMAC_SECRET. Generate once and
# persist so restarts keep the same secret (existing tokens stay valid).
ensure_auth_secret() {
  local secret_file="$DEMO_DIR/auth.secret"
  if [[ ! -s "$secret_file" ]]; then
    if command -v openssl >/dev/null; then
      openssl rand -hex 32 > "$secret_file"
    else
      head -c 32 /dev/urandom | xxd -p -c 64 > "$secret_file"
    fi
    chmod 600 "$secret_file"
    echo "==> generated $secret_file (AUTH_HMAC_SECRET for auth-rbac)"
  fi
  AUTH_HMAC_SECRET="$(cat "$secret_file")"
  export AUTH_HMAC_SECRET
}
```

- [ ] **Step 3: Update the build-step header comment count**

In `harness/scripts/demo.sh:11`, change `cargo build --release for each of the 23 workers + harness` to `cargo build --release for each of the 14 workers + harness`.

In `harness/scripts/demo.sh:67`, change `building harness + ${#WORKERS[@]} dep workers (release)...` — this line uses `${#WORKERS[@]}` so it auto-updates; no edit needed. Keep as-is.

- [ ] **Step 4: Smoke-test the script can still parse**

Run: `bash -n harness/scripts/demo.sh && echo OK`
Expected: `OK` (parse-clean).

- [ ] **Step 5: Commit**

```bash
git add harness/scripts/demo.sh
git -c commit.gpgsign=false commit -m "refactor(harness): slim demo.sh WORKERS list and drop AUTH_HMAC_SECRET"
```

---

## Task 4: Update `harness/ARCHITECTURE.md`

**Files:**
- Modify: `harness/ARCHITECTURE.md` (multiple edits across the file)

The architecture doc enumerates 22 workers in several places. Update to the 14-worker reality.

- [ ] **Step 1: Update the opening summary**

In `harness/ARCHITECTURE.md:3`, replace:

```
The `harness` is a meta-worker for the [iii](https://github.com/iii-experimental/harness) bus. It does not implement chat, agents, or providers itself — it **composes** ~22 specialized workers into a runnable chat surface, exposes a small browser-facing HTTP bridge, and ships a Vite/React UI that talks to the bus through that bridge.
```

with:

```
The `harness` is a meta-worker for the [iii](https://github.com/iii-experimental/harness) bus. It does not implement chat, agents, or providers itself — it **composes** 14 specialized workers into a runnable chat surface, exposes a small browser-facing HTTP bridge, and ships a Vite/React UI that talks to the bus through that bridge.
```

- [ ] **Step 2: Update the bus-diagram bus-line that mentions cut workers**

In `harness/ARCHITECTURE.md:27`, replace:

```
            │  agent::before_tool_call (topic)                   │◄── policy-denylist, audit-log, dlp-scrubber
```

with:

```
            │  agent::before_tool_call (topic)                   │◄── policy-denylist
```

In `harness/ARCHITECTURE.md:28`, replace:

```
            │  auth::* skills::register …                        │◄── auth-credentials, skills, …
```

with:

```
            │  auth::* skills::register …                        │◄── auth-credentials, skills
```

- [ ] **Step 3: Update the section heading "The 22 expected workers"**

In `harness/ARCHITECTURE.md:60`, replace:

```
### 3. The 22 expected workers
```

with:

```
### 3. The 14 expected workers
```

- [ ] **Step 4: Replace the role table at lines 64-74**

In `harness/ARCHITECTURE.md:64-74`, replace the existing Group/Workers/Role table with:

```
| Group | Workers | Role |
|---|---|---|
| Orchestration | `turn-orchestrator`, `provider-router` | Runs a turn end-to-end: fan a request to a provider and dispatch tool calls. |
| Sessions / state | `session-tree`, `session-inbox` | Persisted message trees and a steering/follow-up inbox queue. |
| Catalog | `models-catalog` | Model metadata. |
| Auth | `auth-credentials` | Provider credentials store. |
| Policy / safety | `policy-denylist`, `llm-budget` | Hook subscriber on `agent::before_tool_call` and budget tracking. |
| Hooks | `hook-fanout` | Generic publish-and-collect primitive. |
| Tools | `shell-bash`, `shell-filesystem`, `subagent` | LLM-callable tool implementations. |
| Providers | `provider-anthropic`, `provider-openai` | Concrete LLM transport workers behind `provider-router`. |
```

- [ ] **Step 5: Update the demo.sh command count**

In `harness/ARCHITECTURE.md:84`, replace:

```
demo.sh build    # cargo build --release for harness + 22 workers
```

with:

```
demo.sh build    # cargo build --release for harness + 14 workers
```

In `harness/ARCHITECTURE.md:87`, replace:

```
demo.sh start    # spawn all 22 workers + harness as nohup processes
```

with:

```
demo.sh start    # spawn all 14 workers + harness as nohup processes
```

- [ ] **Step 6: Update the "expected_workers": [ … 22 … ] reference**

In `harness/ARCHITECTURE.md:142`, replace:

```
  "expected_workers": [ … 22 … ],
```

with:

```
  "expected_workers": [ … 14 … ],
```

- [ ] **Step 7: Update the "Drift detection via test" line if it references 22**

Inspect `harness/ARCHITECTURE.md:180` ("Drift detection via test."). If it references "22" specifically, change to "14"; otherwise leave as-is. Run: `/usr/bin/grep -n "22" harness/ARCHITECTURE.md` and edit any remaining occurrences that refer to the worker count (skip mentions of TCP ports, dates, etc.).

- [ ] **Step 8: Commit**

```bash
git add harness/ARCHITECTURE.md
git -c commit.gpgsign=false commit -m "docs(harness): update ARCHITECTURE.md for 14-worker slim list"
```

---

## Task 5: Bump `harness/Cargo.toml` version

**Why:** `EXPECTED_WORKERS` is part of `harness::status` output. Cutting eight names is a breaking change for consumers reading that response, so bump to `0.2.0` per semver.

**Files:**
- Modify: `harness/Cargo.toml:3`

- [ ] **Step 1: Update the version**

In `harness/Cargo.toml:3`, replace:

```toml
version = "0.1.0"
```

with:

```toml
version = "0.2.0"
```

- [ ] **Step 2: Verify the crate still builds**

Run: `cd harness && cargo build --release 2>&1 | tail -5`
Expected: `Finished release [...] target(s)` with no errors.

- [ ] **Step 3: Commit**

```bash
git add harness/Cargo.toml
git -c commit.gpgsign=false commit -m "chore(harness): bump version to 0.2.0 (slim EXPECTED_WORKERS is breaking)"
```

---

## Task 6: Delete the eight cut harness workers

**Why:** These crates have no remaining consumer: each has its own self-contained workspace and is only referenced from `EXPECTED_WORKERS` (already updated) and `demo.sh WORKERS` (already updated).

**Crates to remove:** `auth-rbac/`, `audit-log/`, `dlp-scrubber/`, `guardrails/`, `session-corpus/`, `document-extract/`, `provider-cli/`, `context-compaction/`.

- [ ] **Step 1: Confirm no kept crate has a path dependency on any of these**

Run:

```bash
/usr/bin/grep -rE 'path = "(\.\./)?(auth-rbac|audit-log|dlp-scrubber|guardrails|session-corpus|document-extract|provider-cli|context-compaction)"' --include="Cargo.toml" \
  /Users/ytallolayon/workspaces/personal/motia/workers/ 2>/dev/null
```

Expected: empty output. If any matches, stop and investigate before continuing.

- [ ] **Step 2: Remove the eight directories**

```bash
git rm -r auth-rbac audit-log dlp-scrubber guardrails session-corpus document-extract provider-cli context-compaction
```

- [ ] **Step 3: Verify the harness still compiles**

Run: `cd harness && cargo build --release 2>&1 | tail -5`
Expected: `Finished release [...] target(s)`.

- [ ] **Step 4: Verify a sample kept worker still compiles**

Run: `cd provider-anthropic && cargo build --release 2>&1 | tail -5`
Expected: `Finished release [...] target(s)`.

- [ ] **Step 5: Commit**

```bash
git -c commit.gpgsign=false commit -m "refactor: remove 8 cut harness workers (audit-log, auth-rbac, dlp-scrubber, guardrails, session-corpus, document-extract, provider-cli, context-compaction)"
```

---

## Task 7: Delete the 19 cut provider crates

**Crates to remove:** `provider-azure-openai/`, `provider-bedrock/`, `provider-cerebras/`, `provider-deepseek/`, `provider-fireworks/`, `provider-google/`, `provider-google-vertex/`, `provider-groq/`, `provider-huggingface/`, `provider-kimi-coding/`, `provider-minimax/`, `provider-mistral/`, `provider-openai-responses/`, `provider-opencode-go/`, `provider-opencode-zen/`, `provider-openrouter/`, `provider-vercel-ai-gateway/`, `provider-xai/`, `provider-zai/`.

- [ ] **Step 1: Confirm no kept crate has a path dependency on any of these**

Run:

```bash
/usr/bin/grep -rE 'path = "(\.\./)?(provider-azure-openai|provider-bedrock|provider-cerebras|provider-deepseek|provider-fireworks|provider-google|provider-google-vertex|provider-groq|provider-huggingface|provider-kimi-coding|provider-minimax|provider-mistral|provider-openai-responses|provider-opencode-go|provider-opencode-zen|provider-openrouter|provider-vercel-ai-gateway|provider-xai|provider-zai)"' \
  --include="Cargo.toml" /Users/ytallolayon/workspaces/personal/motia/workers/ 2>/dev/null
```

Expected: empty output. The vendored `crates/` inside each provider use relative paths within their own workspace, which the regex above tolerates because it requires no leading path or one `../` (and the vendored crates are referenced as `crates/...`, not by parent dir name).

- [ ] **Step 2: Remove the 19 directories**

```bash
git rm -r provider-azure-openai provider-bedrock provider-cerebras provider-deepseek provider-fireworks provider-google provider-google-vertex provider-groq provider-huggingface provider-kimi-coding provider-minimax provider-mistral provider-openai-responses provider-opencode-go provider-opencode-zen provider-openrouter provider-vercel-ai-gateway provider-xai provider-zai
```

- [ ] **Step 3: Verify both kept providers still compile**

Run: `cd provider-anthropic && cargo build --release 2>&1 | tail -5 && cd ../provider-openai && cargo build --release 2>&1 | tail -5`
Expected: both finish with `Finished release [...] target(s)`.

- [ ] **Step 4: Verify the harness still compiles**

Run: `cd harness && cargo build --release 2>&1 | tail -5`
Expected: `Finished release [...] target(s)`.

- [ ] **Step 5: Commit**

```bash
git -c commit.gpgsign=false commit -m "refactor: remove 19 unused provider crates (keep anthropic + openai only)"
```

---

## Task 8: Delete `iii-mcp-engine`

**Why:** Recently scaffolded but not part of the slim harness. Per spec, removed at this pass; can be reintroduced later.

- [ ] **Step 1: Confirm no consumer**

Run: `/usr/bin/grep -rE 'path = "(\.\./)?iii-mcp-engine"' --include="Cargo.toml" /Users/ytallolayon/workspaces/personal/motia/workers/ 2>/dev/null`
Expected: empty output.

- [ ] **Step 2: Remove the directory**

```bash
git rm -r iii-mcp-engine
```

- [ ] **Step 3: Check if any worker manifest still registers this skill**

Recent commit `684e094` added self-registration logic. Check whether any kept worker references `iii-mcp-engine` as a registered skill name:

```bash
/usr/bin/grep -rn "iii-mcp-engine\|iii_mcp_engine" \
  --include="*.rs" --include="*.yaml" --include="*.toml" \
  /Users/ytallolayon/workspaces/personal/motia/workers/ 2>/dev/null \
  | grep -v target
```

Expected: empty output. If any references appear, remove them in this same task.

- [ ] **Step 4: Commit**

```bash
git -c commit.gpgsign=false commit -m "refactor: remove iii-mcp-engine crate"
```

---

## Task 9: Update repo-level `README.md`

**Files:**
- Modify: `README.md` (the worker table — drop rows for removed crates)

The README has a table at lines 12-60 enumerating every worker. Remove the rows for the 28 deleted crates.

- [ ] **Step 1: Inspect the current table**

Run: `/usr/bin/sed -n '10,65p' README.md`
Expected: a markdown table with rows like `| [`<name>`](<name>/) | Rust | <description> |`.

- [ ] **Step 2: Delete rows for the 28 cut crates**

Open `README.md` in the editor. Find and delete these table rows (each is a single line beginning `| [\``):

- `audit-log`
- `auth-rbac`
- `context-compaction`
- `dlp-scrubber`
- `document-extract`
- `guardrails`
- `provider-azure-openai`
- `provider-bedrock`
- `provider-cerebras`
- `provider-cli`
- `provider-deepseek`
- `provider-fireworks`
- `provider-google`
- `provider-google-vertex`
- `provider-groq`
- `provider-huggingface`
- `provider-kimi-coding`
- `provider-minimax`
- `provider-mistral`
- `provider-openai-responses`
- `provider-opencode-go`
- `provider-opencode-zen`
- `provider-openrouter`
- `provider-vercel-ai-gateway`
- `provider-xai`
- `provider-zai`
- `session-corpus`
- (note: `iii-mcp-engine` may or may not be in the table; remove if present)

- [ ] **Step 3: Verify no stale links**

Run: `/usr/bin/grep -nE "audit-log|auth-rbac|dlp-scrubber|guardrails|session-corpus|document-extract|provider-cli|context-compaction|iii-mcp-engine|provider-azure-openai|provider-bedrock|provider-cerebras|provider-deepseek|provider-fireworks|provider-google|provider-groq|provider-huggingface|provider-kimi|provider-minimax|provider-mistral|provider-openai-responses|provider-opencode|provider-openrouter|provider-vercel|provider-xai|provider-zai" README.md`

Expected: empty output (or only matches that are intentional historical mentions, e.g. inside a CHANGELOG-style section if one exists).

- [ ] **Step 4: Commit**

```bash
git add README.md
git -c commit.gpgsign=false commit -m "docs: drop removed crates from README worker table"
```

---

## Task 10: Update `TODOS.md` to drop entries for cut workers

**Files:**
- Modify: `TODOS.md`

`TODOS.md:3-5` references `audit-log` (which is being removed). Strip any TODO sections that target a cut crate.

- [ ] **Step 1: Find sections referencing cut crates**

Run: `/usr/bin/grep -nE "audit-log|auth-rbac|dlp-scrubber|guardrails|session-corpus|document-extract|provider-cli|context-compaction" TODOS.md`
Expected: zero or more matches. Each match is inside a `## <heading>` section.

- [ ] **Step 2: Delete each section that targets a cut crate**

For each match from Step 1, open `TODOS.md` and remove the entire `## <heading>` block (heading line through the next `## ` or EOF). If a section's text only incidentally mentions the crate (e.g. "see audit-log for an example") but is otherwise about a kept crate, leave the section but remove the stale reference.

- [ ] **Step 3: Verify no stale references remain**

Run: `/usr/bin/grep -nE "audit-log|auth-rbac|dlp-scrubber|guardrails|session-corpus|document-extract|provider-cli|context-compaction" TODOS.md`
Expected: empty output.

- [ ] **Step 4: Commit**

```bash
git add TODOS.md
git -c commit.gpgsign=false commit -m "docs: drop TODOS entries targeting removed crates"
```

---

## Task 11: Update `AGENTS-NEW-WORKER.md`

**Files:**
- Modify: `AGENTS-NEW-WORKER.md:278`

This file uses `document-extract` as an example of a single-function worker. Replace with a kept example.

- [ ] **Step 1: Inspect the current line**

Run: `/usr/bin/sed -n '275,282p' AGENTS-NEW-WORKER.md`
Expected: a sentence like `If a worker exposes only one function (e.g. document-extract), skip the ...`.

- [ ] **Step 2: Replace `document-extract` with a kept worker that has one function**

A kept single-function-ish worker is `policy-denylist` (one subscriber). Replace `document-extract` with `policy-denylist` on line 278.

- [ ] **Step 3: Confirm no other references to cut crates**

Run: `/usr/bin/grep -nE "audit-log|auth-rbac|dlp-scrubber|guardrails|session-corpus|document-extract|provider-cli|context-compaction" AGENTS-NEW-WORKER.md`
Expected: empty output.

- [ ] **Step 4: Commit**

```bash
git add AGENTS-NEW-WORKER.md
git -c commit.gpgsign=false commit -m "docs: replace document-extract example with policy-denylist"
```

---

## Task 12: End-to-end verification — build everything

- [ ] **Step 1: Build the harness and every kept worker via `demo.sh build`**

Run:

```bash
cd harness && ./scripts/demo.sh build 2>&1 | tail -20
```

Expected: each of the 14 workers + harness reports `[build] <name>` with no errors. Final line: `==> all binaries built.`

- [ ] **Step 2: Confirm no orphan target binary remains for cut workers**

Run: `ls /Users/ytallolayon/workspaces/personal/motia/workers/{auth-rbac,audit-log,dlp-scrubber,guardrails,session-corpus,document-extract,provider-cli,context-compaction,iii-mcp-engine}/target/release/iii-* 2>/dev/null | wc -l`
Expected: `0` (the directories no longer exist, so glob expands to nothing).

---

## Task 13: End-to-end verification — runtime smoke test

- [ ] **Step 1: Start the engine**

```bash
cd harness && ./scripts/demo.sh engine
```

Expected: `==> engine ready after Ns (pid <pid>)`.

- [ ] **Step 2: Spawn the 14 workers + harness**

```bash
cd harness && ./scripts/demo.sh start
```

Expected: 15 lines of `[start] <name> pid=<pid> log=...` (14 deps + harness). The output should mention exactly 14 dep workers.

- [ ] **Step 3: Verify harness::status returns the slim list**

```bash
cd harness && ./scripts/demo.sh verify
```

Expected: the `harness::status` JSON response contains `"expected_workers": [ ... 14 entries ... ]` matching the array in `lib.rs`. `models::list` returns at least one model entry. `provider::cli::list_models` will likely error since `provider-cli` is gone — that is **expected**; the verify command should not require it. If `verify` fails *only* on the `provider-cli` line, that is acceptable. If it fails earlier, stop and investigate.

- [ ] **Step 4: (Optional) Update `cmd_verify` to drop the `provider-cli` probe**

If Step 3 reported an error specifically on `provider::cli::list_models`, edit `harness/scripts/demo.sh` `cmd_verify` (around line 136) to remove the last block:

```bash
echo
echo "==> provider::cli::list_models (proves provider-cli connected)"
iii --use-default-config trigger --function-id provider::cli::list_models || true
```

Then run `./scripts/demo.sh verify` again. Expected: clean output with no error lines.

If you applied this edit, commit it:

```bash
git add harness/scripts/demo.sh
git -c commit.gpgsign=false commit -m "refactor(harness): drop provider-cli probe from demo verify"
```

- [ ] **Step 5: Bring up the UI**

```bash
cd harness && ./scripts/demo.sh web
```

Expected: `==> http://localhost:5173`. Open the URL in a browser.

- [ ] **Step 6: Manual chat smoke test**

In the browser:

1. Open the AuthPanel and set an `ANTHROPIC_API_KEY` (or `OPENAI_API_KEY`).
2. Send a message: `list files in /tmp`.
3. Expect: model returns text, may invoke `shell::filesystem::ls`. Tool result appears inline. Final assistant text mentions the listing.

If the chat completes successfully, the slim harness is working.

- [ ] **Step 7: Manual policy denial smoke test**

Stop the stack: `cd harness && ./scripts/demo.sh stop`. Restart with a denylist (the env var must propagate to the spawned worker, so set it for the whole shell session before invoking `start`):

```bash
cd harness
export POLICY_DENIED_TOOLS="shell::filesystem::write"
./scripts/demo.sh start
```

Re-open the UI, send: `write "hello" to /tmp/test.txt`. Expect: tool result is a `blocked by policy` error and the assistant explains the denial. This proves `policy-denylist` is still wired up.

- [ ] **Step 8: Stop the stack**

```bash
cd harness && ./scripts/demo.sh stop
```

Expected: clean shutdown of all PIDs and the engine.

---

## Task 14: Final repo-wide stale-reference sweep

- [ ] **Step 1: Repo-root grep for live references to any cut crate**

Run:

```bash
/usr/bin/grep -rnE "auth-rbac|audit-log|dlp-scrubber|guardrails|session-corpus|document-extract|provider-cli|context-compaction|iii-mcp-engine|provider-azure-openai|provider-bedrock|provider-cerebras|provider-deepseek|provider-fireworks|provider-google|provider-groq|provider-huggingface|provider-kimi|provider-minimax|provider-mistral|provider-openai-responses|provider-opencode|provider-openrouter|provider-vercel|provider-xai|provider-zai" \
  --include="*.rs" --include="*.toml" --include="*.yaml" --include="*.json" --include="*.sh" \
  /Users/ytallolayon/workspaces/personal/motia/workers/ 2>/dev/null \
  | grep -v "target/" \
  | grep -v "Cargo.lock"
```

Expected: empty output. Anything that prints is a leftover live reference. Investigate each and fix before proceeding.

- [ ] **Step 2: Spec design-doc trace stays untouched**

The design spec at `docs/superpowers/specs/2026-05-06-slim-harness-design.md` legitimately names every cut crate as part of the cut list. That is intentional historical documentation; do **not** edit it.

- [ ] **Step 3: If Step 1 found and fixed any stale references, commit**

```bash
git -c commit.gpgsign=false commit -am "chore: clean up final stale references to removed crates"
```

(Skip if Step 1 was already empty.)

---

## Task 15: Final review checklist

Walk through each item below before declaring the slim harness done. No new code; this is an audit.

- [ ] `harness/src/lib.rs:18` shows the 14-worker `EXPECTED_WORKERS` list.
- [ ] `harness/iii.worker.yaml` exists and has 14 dependency entries.
- [ ] `harness/scripts/demo.sh` `WORKERS=( ... )` lists 14 entries; `ensure_auth_secret` is gone.
- [ ] `harness/ARCHITECTURE.md` says "14 specialized workers", the role table reflects the slim list, and bus diagram drops `audit-log`/`dlp-scrubber`.
- [ ] `harness/Cargo.toml` version is `0.2.0`.
- [ ] `cargo test -p iii-harness` passes including `expected_workers_matches_yaml_dependency_count`.
- [ ] `./scripts/demo.sh build && engine && start && verify` succeeds end-to-end (with optional `provider-cli` probe removed).
- [ ] Browser smoke test: `list files in /tmp` returns a model reply with a tool result.
- [ ] Policy denial smoke test: a denied tool returns a `blocked by policy` error.
- [ ] No `git rm`-able crate directories remain among the 28 names listed in the spec.
- [ ] Repo-wide grep in Task 14 Step 1 returns empty.

---

## Notes for the executor

**Order matters.** Tasks 1–5 prepare the harness for the 14-worker world *before* any crate is deleted. Tasks 6–8 delete crates, which is when we discover any orphan path dependencies. Tasks 9–11 mop up tracked docs. Tasks 12–14 are verification.

**Per-task commit discipline.** Each task ends with one commit. Do not collapse commits — a small, reversible commit per logical step makes any rollback trivial.

**Do not skip the smoke tests in Task 13.** A green `cargo build` is necessary but not sufficient: the harness boots a multi-process system and the only way to know it works is to run a real chat turn through the UI.

**If a kept worker fails to build.** Investigate before plowing forward. The vendored `crates/` inside `provider-anthropic/` and `provider-openai/` are independent copies, so deletions elsewhere should not affect them. If they do, something in the spec's "no cross-tree path dependency" assumption was wrong — stop and report.
