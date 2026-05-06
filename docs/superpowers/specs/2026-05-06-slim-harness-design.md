# Slim Harness Design

**Date:** 2026-05-06
**Status:** Draft for review
**Owners:** Ytallo Layon

## Summary

Trim the `harness` meta-worker from a 22-worker bundle to a **14-worker** bundle that boots cleanly, lets the UI chat with Anthropic and OpenAI, and keeps the policy trust boundary (`policy-denylist`) plus cost observability (`llm-budget`) intact. Delete **28 crates** from the repo that no consumer needs after the trim. The `harness/` crate stays a meta-worker — same architecture as today, just a shorter `EXPECTED_WORKERS` list and a recreated `iii.worker.yaml`.

## Goals

- Smallest demo with working tools: UI loads, chats with Anthropic and OpenAI, runs `shell::filesystem::*`, `shell::bash::*`, and `subagent::*` tools.
- Keep the engine-side trust boundary (`policy-denylist`) so destructive shell ops can still be blocked by env-configured deny rules.
- Keep cost observability (`llm-budget`) so the existing `CostPanel` UI continues to work.
- Repo footprint shrinks: 28 unused crates removed.

## Non-goals

- Patching providers to read API keys directly from env vars. `auth-credentials` stays as the credential store; `provider-anthropic` and `provider-openai` are unchanged.
- Merging the three tool workers (`shell-bash`, `shell-filesystem`, `subagent`) into one `tools` worker.
- Hoisting the duplicated internal `crates/` directories (`provider-base`, `harness-types`, `auth-credentials`, `overflow-classify`) currently vendored inside every provider crate into one shared workspace. That dedup is a separate refactor.
- Touching CI/CD pipeline configurations under `.github/` unless a workflow explicitly enumerates a cut crate.

## The 14 kept workers

Current `EXPECTED_WORKERS` in `harness/src/lib.rs:18` enumerates 22 names. The slim list below replaces it.

| # | Worker | Role |
|---|---|---|
| 1 | `turn-orchestrator` | Drives every agent turn (`run::start_and_wait`, `turn::*`) |
| 2 | `provider-router` | Single entry in front of providers (`router::generate`, `router::push_steering`, `router::push_followup`, `router::abort`) |
| 3 | `session-tree` | Persistent message tree (`session::create/append/tree/messages/clone/fork/compact/export_html`) |
| 4 | `session-inbox` | Steering/follow-up FIFO (`inbox::push/peek/drain`); used by `provider-router/src/register.rs:450` and `turn-orchestrator/src/states/steering.rs:135` |
| 5 | `models-catalog` | Model metadata (`models::list`, `models::get`) |
| 6 | `hook-fanout` | Generic publish-and-collect for `agent::*` events |
| 7 | `policy-denylist` | Trust boundary on `agent::before_tool_call` (configured via `POLICY_DENIED_TOOLS`) |
| 8 | `shell-bash` | LLM-callable shell under `shell::bash::*` |
| 9 | `shell-filesystem` | LLM-callable file ops under `shell::fs::*` / `shell::filesystem::*` |
| 10 | `subagent` | Spawn child agent sessions under `subagent::*` |
| 11 | `provider-anthropic` | Anthropic Messages API (`provider::anthropic::generate`) |
| 12 | `provider-openai` | OpenAI Chat Completions API (`provider::openai::generate`) |
| 13 | `auth-credentials` | Provider keys via `auth::set_token` / `auth::get_token`; load-bearing because `provider-openai-responses/crates/provider-base/src/auth.rs:21` calls `auth::get_token` from every provider's registration path |
| 14 | `llm-budget` | Backs `CostPanel.tsx` (`budget::list`, `budget::usage`, `budget::forecast`, `budget::create`) |

## Cut workers (8 from `EXPECTED_WORKERS`)

| Worker | Reason for removal |
|---|---|
| `auth-rbac` | RBAC not needed for single-tenant local demo |
| `audit-log` | Audit trail not needed for slim demo |
| `dlp-scrubber` | DLP scanning not needed for slim demo; no consumer |
| `guardrails` | Policy hook not needed for slim demo; no consumer |
| `session-corpus` | Misnamed — actually a dataset export pipeline (`corpus::scan/redact/review/publish`), not chat-session storage; no caller in `harness/`, `turn-orchestrator/`, or `provider-router/` |
| `document-extract` | Document → text extraction; not needed unless UI ships file upload |
| `provider-cli` | Outside the "Anthropic + OpenAI only" provider scope |
| `context-compaction` | Passive subscriber on `agent::events`; verified no caller in core path. Removing it means long sessions don't auto-compact and will fail with a provider error if they exceed context. Acceptable for slim demo; user can call `session::compact` manually |

## Crate deletions (28 total)

### 8 cut harness workers
`auth-rbac/`, `audit-log/`, `dlp-scrubber/`, `guardrails/`, `session-corpus/`, `document-extract/`, `provider-cli/`, `context-compaction/`

### 19 cut provider crates
All non-Anthropic/non-OpenAI providers, plus the OpenAI Responses API client (newer API, not used by current UI/router):
`provider-azure-openai/`, `provider-bedrock/`, `provider-cerebras/`, `provider-deepseek/`, `provider-fireworks/`, `provider-google/`, `provider-google-vertex/`, `provider-groq/`, `provider-huggingface/`, `provider-kimi-coding/`, `provider-minimax/`, `provider-mistral/`, `provider-openai-responses/`, `provider-opencode-go/`, `provider-opencode-zen/`, `provider-openrouter/`, `provider-vercel-ai-gateway/`, `provider-xai/`, `provider-zai/`

Each of these vendors its own `crates/` subdirectory (`auth-credentials`, `harness-types`, `overflow-classify`, `provider-base`). Verified by `grep` that no out-of-tree consumer points at any of them — these copies are local to each provider workspace. They are removed with the parent crate.

### 1 other
`iii-mcp-engine/` — not in `EXPECTED_WORKERS`, scaffolded recently in commits `985eb92`, `ff5bc2b`, `684e094`, but not part of the slim harness scope.

## Untouched (explicit non-targets)

These crates stay in the repo because they have other consumers or are independent infrastructure:

`registry/`, `mcp/`, `autoharness/`, `iii-database/`, `iii-lsp/`, `iii-lsp-vscode/`, all `oauth-*` crates, `image-resize/`, `state-flag/`, `proof/`, `sensor/`, `shell/`, `skills/`, `todo-worker/`, `todo-worker-python/`.

The vendored `crates/` inside `provider-anthropic/` and `provider-openai/` (their copies of `provider-base`, `harness-types`, `auth-credentials`, `overflow-classify`) stay duplicated. Deduping into one shared workspace is a separate refactor.

## Edits inside `harness/`

| File | Change |
|---|---|
| `harness/src/lib.rs:18` | Replace 22-entry `EXPECTED_WORKERS` array with the 14-entry list above |
| `harness/iii.worker.yaml` | **Recreate** the missing manifest (per `ARCHITECTURE.md:77`) with 14 dependency entries; mirror schema from `provider-openai/iii.worker.yaml` |
| `harness/scripts/demo.sh:32-40` | Update `WORKERS=( ... )` to the 14 names; remove the `ensure_auth_secret` function and the `AUTH_HMAC_SECRET` plumbing (was needed only by `auth-rbac`) |
| `harness/tests/integration.rs:7` | No code change. The test asserts `EXPECTED_WORKERS.len() == yaml deps`; passes automatically once both lists are 14 |
| `harness/ARCHITECTURE.md` | Update worker count ("22 workers" → "14"), the role table at line 65, and any examples that name cut workers (`audit-log`, `dlp-scrubber`, `guardrails` on line 27) |
| `harness/Cargo.toml` | Bump version `0.1.0` → `0.2.0` (semver: `EXPECTED_WORKERS` shape change is breaking for `harness::status` readers) |

## UI side

No UI changes. `CostPanel.tsx` keeps working because `llm-budget` is kept. `AuthPanel.tsx` keeps working because `auth-credentials` is kept. `App.tsx`'s tool advertisements (`shell::filesystem::*`) point at `shell-filesystem` which is kept.

## Definition of done

1. `harness/src/lib.rs` compiles with the new 14-entry `EXPECTED_WORKERS`.
2. `cargo test -p iii-harness` passes — including the existing `expected_workers_matches_yaml_dependency_count` integration test.
3. `harness/scripts/demo.sh build && demo.sh engine && demo.sh start && demo.sh verify` runs to completion locally and `harness::status` returns the 14-name `expected_workers` array.
4. `harness/scripts/demo.sh web` brings up the UI; user can pick Anthropic or OpenAI, send a message, get a reply, run a `shell::filesystem::ls` tool call, and observe a denial when triggering a tool listed in `POLICY_DENIED_TOOLS`.
5. `cargo build --workspace` (or equivalent) at the repo root passes with the 28 crates removed — no orphan path dependencies.
6. Repo root `grep` for the 28 cut crate names returns only documentation/historical mentions (e.g. CHANGELOG entries), no live code or manifest references.

## Risks

- **Recreating `iii.worker.yaml` from scratch.** No existing harness manifest to crib from. Will mirror the schema another worker uses (e.g. `provider-openai/iii.worker.yaml`). If the engine has a stricter schema we will find out at boot.
- **`HARNESS-WORKER-PUBLISHING-MAP.md`** at the repo root may list every worker by name; if so, removing 28 crates means stale rows. Will check and update or document as known stale.
- **No auto-compaction.** Sessions that exceed model context will fail with a provider error rather than recover. Mitigation: keep demo sessions short, or call `session::compact` from the UI manually.
- **Recent `iii-mcp-engine` integration.** Commit `684e094` ("self-register with skills worker on boot") wired this crate into a manifest. Need to undo that registration when deleting the crate.
- **Duplicate crate names across the repo.** Several cut providers vendor crates with the same names as kept ones (e.g. each has its own `crates/auth-credentials/`). Deletion is by directory, not by crate name; verified no Cargo path dependency crosses provider boundaries.

## Open questions

None. All design decisions confirmed with user during brainstorming session 2026-05-06.

## Trace of decisions

- Goal: option B "smallest demo with working tools" — keeps `policy-denylist`.
- Auth: option A "keep `auth-credentials`" — providers' registration path requires `auth::get_token`; patching them is out of scope.
- Cleanup scope: option B "slim harness AND delete unused crates from the workers repo."
- OpenAI Responses API: option A "drop `provider-openai-responses` entirely" — slim, current UI/router use Chat Completions.
- Cost observability: option B "keep `llm-budget`" — preserves existing `CostPanel.tsx`.
- Auto-compaction: removed at user request after confirming no hard-coupled caller in the core path.
