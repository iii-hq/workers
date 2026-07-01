# provider-openai-codex

OpenAI **Codex (ChatGPT subscription)** provider worker behind
[llm-router](../llm-router/). It lets the stack generate against a ChatGPT/Codex
subscription (billed to the plan, "Sign in with ChatGPT") instead of a
pay-per-token API key, by speaking OpenAI's **Responses API** at the Codex
backend.

Implements the provider protocol from `tech-specs/2026-06-agentic/llm-router.md`:
`provider::openai-codex::stream` (Responses SSE → `AssistantMessageEvent` frames
into a router-owned channel) and `provider::openai-codex::refresh_models`
(reconciles a static, namespaced catalog slice).

> ⚠️ **Terms-of-service caveat — local/personal dev only.** This drives a
> personal ChatGPT subscription through the undocumented
> `https://chatgpt.com/backend-api/codex` backend with a Codex-client
> `originator` header. That is plausibly against OpenAI's terms and may risk
> account action; the backend is unversioned and can change without notice. Use
> it for local, personal development. For team/CI/production, use official
> API-key billing (`provider-openai`). You assume the risk.

## Credentials

This worker is a **dumb token consumer** — login and refresh live out-of-band:

1. **Vault (intended authority):** the [`auth-credentials`](../auth-credentials/)
   worker (`auth::get_token`), populated by the `oauth-openai-codex` "Sign in
   with ChatGPT" flow. A near-expiry token triggers the vault-owned refresh
   (`oauth::openai-codex::refresh`); this provider never calls the OAuth
   endpoints itself.
2. **Local dev fallback:** when no vault is running, the worker reads
   `${CODEX_HOME:-$HOME/.codex}/auth.json` directly (read-only — the `codex`
   CLI owns that file's refresh). Requires host access to that path, so it does
   not apply to sandboxed/microVM-managed workers. On boot the worker also does
   a one-time, read-only import of that file into the vault when the vault is
   present but empty (never written back).

API-key credentials are rejected — they belong on `provider-openai` under
provider id `openai`.

## Behavior

- **Registration:** self-declares via `router::provider::register` with backoff,
  and re-declares on the `router::ready` trigger. The declaration ships a static,
  **namespaced** `models` slice and `credential_env_var: None`. Identity binds
  via the `registration_token` persisted in iii-state (scope
  `provider-openai-codex`).
- **Models:** the ChatGPT backend has no usable `GET /v1/models`, so the catalog
  is a static slice with **namespaced** router ids (`codex/gpt-5.5`,
  `codex/gpt-5.4`, `codex/gpt-5.4-mini`, `codex/gpt-5-codex`) mapped to the
  upstream ids on the wire. Namespacing guarantees no `AmbiguousModel` collision
  with `provider-openai`'s `gpt-5.*` catalog. Select a Codex model by its
  `codex/*` id (or pin `provider: "openai-codex"`).
- **Request:** Responses API — `input` items, `stream: true`, `store: false`,
  optional `tools` and `reasoning: { effort }`. Headers: `Authorization: Bearer`,
  `chatgpt-account-id`, `openai-beta: responses=experimental`,
  `originator: codex_cli_rs`.
- **SSE:** `response.output_text.delta` → text, `response.reasoning_*` →
  thinking, `response.function_call_arguments.delta` → tool calls,
  `response.completed` → usage + terminal. Unknown event types are ignored
  (forward-compat).
- **Liveness / errors:** `ping` at least every 30s of silence; 401/403 →
  `auth_expired`, 429 → `rate_limited`, `context_length_exceeded` →
  `context_overflow`, 5xx/network → `transient`, other 4xx → `permanent`. The
  router owns retry policy.

## Running

Standard worker CLI: `--url` (engine WebSocket, default `ws://127.0.0.1:49134`,
or `III_WS_URL`), `--manifest` (print the registry manifest and exit), `--config`
(accepted but ignored — this worker has no file-based config).

```bash
cargo run -- --url ws://127.0.0.1:49134
```

## Tests

```bash
cargo test    # unit modules (config/auth/wire/sse/curated), upstream TCP stubs, schema goldens
```

Regenerate the wire-schema goldens with `UPDATE_GOLDENS=1 cargo test`.

## Troubleshooting

| Symptom | Cause | Fix |
| --- | --- | --- |
| `not configured: sign in with ChatGPT …` | no vault credential and no readable `~/.codex/auth.json` | run the `oauth-openai-codex` sign-in, or `codex login` so `~/.codex/auth.json` exists |
| `vault import failed (function_not_found: auth::set_token)` | `auth-credentials` vault not running | start the vault, or rely on the local `~/.codex/auth.json` fallback |
| `requires a ChatGPT OAuth login … API keys belong on provider-openai` | credential is an API key | this provider is OAuth-only; use `provider-openai` for keys |
| `missing ChatGPT account id` | token lacks the account claim | sign in again with a ChatGPT account |
| backend `Unsupported parameter` / shape errors | Codex backend contract drifted | update this worker's request/SSE mapping against the current backend |
| model routes ambiguously | a `codex/*` id collided with another provider | keep codex ids namespaced; or pin `provider: "openai-codex"` |
