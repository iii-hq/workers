# Harness Worker Publishing Map

This document defines how the Rust workers currently living in
`motia/harness/workers` should be prepared for publication from this repository.

The target repository publishes one worker per root-level directory. Each
published worker must be self-contained and satisfy `AGENTS-NEW-WORKER.md`:

- `<worker>/iii.worker.yaml`
- `<worker>/README.md`
- `<worker>/Cargo.toml`
- `<worker>/src/main.rs`
- `<worker>/tests/`
- a binary deploy entry in `iii.worker.yaml`

The harness workspace is not shaped that way today. It is a Rust workspace with
many library crates, shared path dependencies, test-only crates, and CLI/TUI
binaries. Do not copy the whole `workers/` tree into this repository. Publish
units should be selected deliberately.

## Packaging Decision

Use two packaging lanes.

1. `harness` as the first all-in-one worker.
   - Source: `workers/harnessd`.
   - Target folder: `harness`.
   - Binary name: `iii-harness`.
   - Purpose: register the durable harness runtime, turn orchestrator, shell
     workers, provider router support, auth, budgets, guardrails, models,
     session helpers, and optional providers in one installable worker.
   - Rationale: proves the release pipeline with the least cross-worker
     dependency churn.

2. Modular workers after the all-in-one worker is released.
   - Source examples: `turn-orchestrator`, `shell-filesystem`, `shell-bash`,
     `subagent`, providers, OAuth workers, `llm-budget`, `auth-rbac`.
   - Target folder: one folder per registry worker.
   - Binary name: `iii-<worker>`.
   - Rationale: lets users install a smaller graph once the shared crate
     boundaries are stable.

## Dependency Strategy

Avoid cross-worker path dependencies in this repository's published workers.
The current release workflow runs in each worker directory and assumes that the
worker is buildable as an independent Rust project.

For the pilot `harness` worker, vendor the required harness library crates under
the `harness/` folder as private path dependencies:

```text
harness/
  Cargo.toml
  iii.worker.yaml
  README.md
  src/main.rs
  crates/
    provider-router/
    harness-types/
    turn-orchestrator/
    hook-fanout/
    session-inbox/
    sandbox-helpers/
    shell-filesystem/
    shell-bash/
    subagent/
    provider-base/
    provider-*/
    oauth-*/
    auth-credentials/
    auth-rbac/
    guardrails/
    llm-budget/
    models-catalog/
    session-tree/
    session-corpus/
    context-compaction/
    document-extract/
    policy-subscribers/
```

This keeps CI and release logic unchanged: the changed worker is still just
`harness`, and `cargo fmt`, `cargo clippy`, and `cargo test` run from that
folder.

After the pilot works, decide whether shared crates should remain vendored per
worker, become published crates, or move into a reusable in-repo support area
with CI/release workflow changes.

## Existing Name Conflicts

These harness crate names already exist as root workers in this repository:

| Harness crate | Existing target folder | Action |
|---|---|---|
| `guardrails` | `guardrails/` | Do not overwrite blindly. Diff behavior and either replace in a dedicated PR or publish under a new name such as `harness-guardrails`. |
| shell family | `shell/` exists | Keep harness sandbox shell workers under explicit names: `shell-filesystem`, `shell-bash`, `subagent`. |
| provider router concepts | `llm-router/` exists | Provider crates are provider adapters, not the same as `llm-router`; publish as `provider-*` if modularized. |

## Worker Classification

### Pilot publish unit

| Target | Source | Deploy | Notes |
|---|---|---|---|
| `harness` | `workers/harnessd` | binary | First release candidate. All-in-one registration process. Rename binary to `iii-harness` for registry consistency. |

### User-facing applications

| Target | Source | Publish? | Notes |
|---|---|---|---|
| `harness-cli` | `workers/harness-cli` | later | Developer/user CLI. Not a background worker by default; publish only if install UX needs it. |
| `harness-tui` | `workers/harness-tui` | later | Interactive TUI. Keep separate from worker registry unless the registry intentionally ships apps. |

### Modular runtime workers

| Target | Source | Deploy | Notes |
|---|---|---|---|
| `turn-orchestrator` | `workers/primitives/turn-orchestrator` | binary | Durable `run::start` state machine. Needs a thin `main.rs` wrapper. |
| `provider-router` | `workers/harness-runtime` | internal or binary | Today it fans in primitives and shells. In modular mode, either publish as `provider-router` or split its registrations into the all-in-one worker only. |
| `hook-fanout` | `workers/primitives/hook-fanout` | internal | Primitive helper; likely vendored, not registry-facing. |
| `session-inbox` | `workers/primitives/session-inbox` | internal | Primitive helper; likely vendored, not registry-facing. |

### Shell workers

| Target | Source | Deploy | Notes |
|---|---|---|---|
| `shell-filesystem` | `workers/shells/shell-filesystem` | binary | Wraps `sandbox::fs::*`; needs `main.rs` wrapper and README. |
| `shell-bash` | `workers/shells/shell-bash` | binary | Wraps `sandbox::exec`; no host fallback. |
| `subagent` | `workers/shells/subagent` | binary | Wraps `run::start` for child sessions. |
| `sandbox-helpers` | `workers/shells/sandbox-helpers` | internal | Shared library for shell workers; vendor under each shell or publish as a crate later. |

### Provider adapters

All provider adapters should share one wrapper template:

- connect to `III_URL` or `--url`
- call `<provider>::register_with_iii(&iii).await`
- wait for `ctrl_c`
- shutdown cleanly

| Target | Source | Deploy | Notes |
|---|---|---|---|
| `provider-anthropic` | `workers/provider-anthropic` | binary | Native Anthropic Messages API. |
| `provider-openai` | `workers/provider-openai` | binary | OpenAI Chat Completions. |
| `provider-openai-responses` | `workers/provider-openai-responses` | binary | OpenAI Responses API. |
| `provider-google` | `workers/provider-google` | binary | Gemini API. |
| `provider-google-vertex` | `workers/provider-google-vertex` | binary | Vertex AI Gemini. |
| `provider-azure-openai` | `workers/provider-azure-openai` | binary | Azure OpenAI Responses shape. |
| `provider-bedrock` | `workers/provider-bedrock` | hold | Stub/error implementation; publish only when intentionally useful. |
| `provider-openrouter` | `workers/provider-openrouter` | binary | OpenAI-compatible path via provider-base. |
| `provider-groq` | `workers/provider-groq` | binary | OpenAI-compatible path via provider-base. |
| `provider-cerebras` | `workers/provider-cerebras` | binary | OpenAI-compatible path via provider-base. |
| `provider-xai` | `workers/provider-xai` | binary | OpenAI-compatible path via provider-base. |
| `provider-deepseek` | `workers/provider-deepseek` | binary | OpenAI-compatible path via provider-base. |
| `provider-mistral` | `workers/provider-mistral` | binary | OpenAI-compatible path via provider-base. |
| `provider-fireworks` | `workers/provider-fireworks` | binary | OpenAI-compatible path via provider-base. |
| `provider-kimi-coding` | `workers/provider-kimi-coding` | binary | OpenAI-compatible path via provider-base. |
| `provider-minimax` | `workers/provider-minimax` | binary | OpenAI-compatible path via provider-base. |
| `provider-zai` | `workers/provider-zai` | binary | OpenAI-compatible path via provider-base. |
| `provider-huggingface` | `workers/provider-huggingface` | binary | OpenAI-compatible path via provider-base. |
| `provider-vercel-ai-gateway` | `workers/provider-vercel-ai-gateway` | binary | OpenAI-compatible path via provider-base. |
| `provider-opencode-zen` | `workers/provider-opencode-zen` | binary | OpenAI-compatible path via provider-base. |
| `provider-opencode-go` | `workers/provider-opencode-go` | binary | OpenAI-compatible path via provider-base. |
| `provider-cli` | `workers/provider-cli` | binary | Depends on `shell::bash::*`; publish after shell workers or bundle in `harness`. |
| `provider-faux` | `workers/provider-faux` | test/internal | Deterministic test provider; publish only as a developer fixture if needed. |
| `provider-base` | `workers/provider-base` | internal | Shared library; do not publish as a worker. |
| `overflow-classify` | `workers/overflow-classify` | internal | Shared provider utility; do not publish as a worker. |

### OAuth workers

Each OAuth crate can publish as a standalone binary once wrapper code is added.

| Target | Source | Deploy | Notes |
|---|---|---|---|
| `oauth-anthropic` | `workers/oauth-anthropic` | binary | PKCE localhost flow. |
| `oauth-openai-codex` | `workers/oauth-openai-codex` | binary | PKCE localhost flow. |
| `oauth-github-copilot` | `workers/oauth-github-copilot` | binary | Device-code flow. |
| `oauth-google-gemini-cli` | `workers/oauth-google-gemini-cli` | binary | PKCE localhost flow. |
| `oauth-google-antigravity` | `workers/oauth-google-antigravity` | binary | PKCE localhost flow. |

### Policy, auth, budget, catalog, and session support

| Target | Source | Deploy | Notes |
|---|---|---|---|
| `auth-credentials` | `workers/auth-credentials` | binary | Provider credential vault; needs storage config in README. |
| `auth-rbac` | `workers/auth-rbac` | binary | HMAC API keys and roles. |
| `llm-budget` | `workers/llm-budget` | binary | Spend caps and records. |
| `models-catalog` | `workers/models-catalog` | binary | Model capability lookup. |
| `context-compaction` | `workers/context-compaction` | binary | Subscriber; publish after session-tree strategy is settled. |
| `session-tree` | `workers/session-tree` | binary or internal | Can be standalone if external sessions need it; otherwise vendor into `harness`. |
| `session-corpus` | `workers/session-corpus` | later | Dataset publishing pipeline; likely standalone but not needed for initial harness release. |
| `document-extract` | `workers/document-extract` | later | Useful standalone worker; needs binary wrapper. |
| `policy-denylist` | `workers/policy-subscribers` | split binary | Existing crate has three binaries. Prefer three target folders or one `policy-subscribers` bundle. |
| `audit-log` | `workers/policy-subscribers` | split binary | Needs idempotency/dedup documentation before publish. |
| `dlp-scrubber` | `workers/policy-subscribers` | split binary | Hook subscriber. |
| `hook-example` | `workers/hook-example` | example/internal | Keep as example unless registry wants samples. |

### Test-only or non-publish crates

| Source | Reason |
|---|---|
| `workers/replay-test` | Integration tests only. |
| `workers/fixtures-gen` | Fixture generation utility, not an iii worker. |
| `workers/harness-types` | Shared data types; internal dependency. |

## Pilot Worker Checklist: `harness`

1. Create `harness/` in this repository.
2. Copy `workers/harnessd/src/main.rs` to `harness/src/main.rs`.
3. Copy required harness libraries into `harness/crates/`.
4. Rewrite `harness/Cargo.toml`:
   - standalone `[package]`
   - `name = "iii-harness"`
   - `version = "0.1.0"`
   - path dependencies under `crates/`
   - external dependencies pinned like existing workers
5. Add `harness/iii.worker.yaml`:

   ```yaml
   iii: v1
   name: harness
   language: rust
   deploy: binary
   manifest: Cargo.toml
   bin: iii-harness
   description: Durable agent harness worker with providers, tools, auth, budgets, and resume support.
   ```

6. Add `harness/README.md`:
   - functions registered
   - provider selection/config
   - resume behavior
   - required companion engine workers, especially sandbox workers if not bundled
7. Add `harness/tests/integration.rs`:
   - at minimum, a config/parser smoke test or direct handler smoke test
   - if engine-gated, also include at least one ungated assertion so CI is meaningful
8. Run from `harness/`:

   ```bash
   cargo fmt --all -- --check
   cargo clippy --all-targets --all-features -- -D warnings
   cargo test --all-features
   ```

9. Add registry metadata only after the binary builds:
   - update `README.md` module table
   - update `registry/index.json`

## Open Decisions

1. Should the registry expose `harness` as the all-in-one default, or should
   users install a modular set such as `turn-orchestrator`, `shell-*`, and
   `provider-*`?
2. Should support crates be vendored per worker, published to crates.io, or
   supported as shared in-repo dependencies by changing CI/release workflows?
3. Should existing `guardrails/` be replaced by the harness version or kept as
   the stable published worker until a compatibility diff is reviewed?
4. Should `harness-cli` and `harness-tui` be registry workers, or separate
   installable apps?
5. Should `policy-subscribers` publish as one bundle or three independent
   workers?

## Recommended Execution Order

1. Land this publishing map.
2. Build the `harness` all-in-one pilot.
3. Validate CI locally in `harness/`.
4. Add registry entry for `harness`.
5. Cut a `harness/v0.1.0` release through Create Tag.
6. Extract modular workers after the all-in-one release is installable.
