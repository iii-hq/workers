# provider-claude-code

Claude **Pro/Max subscription** provider worker behind
[llm-router](https://github.com/iii-hq/workers/tree/main/llm-router). It lets the
stack generate against a personal Claude Code subscription (billed to the plan)
instead of a pay-per-token API key, by reusing the OAuth credentials the Claude
Code CLI already stores locally and speaking Anthropic's **Messages API** with a
Bearer token. It is the Claude analog of
[`provider-openai-codex`](../provider-openai-codex/).

Implements the provider protocol from `tech-specs/2026-06-agentic/llm-router.md`:
`provider::claude-code::stream` (Messages SSE → `AssistantMessageEvent` frames
into a router-owned channel) and `provider::claude-code::refresh_models` (fetches
and reconciles the model catalog, with a curated fallback). For **API-key**
billing (teams/CI/production), use [`provider-anthropic`](../provider-anthropic/)
instead — that provider speaks the same API with `ANTHROPIC_API_KEY`.

> ⚠️ **Terms-of-service caveat — local/personal dev only.** This drives a
> personal Claude Pro/Max subscription outside the official Claude Code CLI: it
> sends the Claude Code identity system prompt and the
> `anthropic-beta: oauth-2025-04-20` header so `api.anthropic.com` accepts the
> subscription OAuth token. That is plausibly against Anthropic's terms and may
> risk account action. Use it for local, personal development. For
> team/CI/production, use official API-key billing (`provider-anthropic`). You
> assume the risk.

## Credentials

This worker is a **dumb token consumer** — login and refresh live out-of-band:

1. **Vault (intended authority):** the [`auth-credentials`](../auth-credentials/)
   worker (`auth::get_token`), populated by an `oauth-claude-code` sign-in flow.
   A near-expiry token triggers the vault-owned refresh
   (`oauth::claude-code::refresh`); this provider never calls the OAuth token
   endpoint itself. That worker/flow is out of scope here — this provider only
   *consumes* and *triggers*. The credential record it expects is
   `{ type: "oauth", access_token, refresh_token?, expires_at?(seconds),
   provider_extra: { subscription_type?, scopes? }, refresh_fn:
   "oauth::claude-code::refresh" }`.
2. **Local dev fallback:** when no vault is running, the worker reads
   `${CLAUDE_CONFIG_DIR:-$HOME/.claude}/.credentials.json` directly (read-only —
   the `claude` CLI owns that file's refresh, including rotating the refresh
   token). The file's `claudeAiOauth.expiresAt` is epoch **milliseconds**;
   it is stored as seconds in the vault shape. Requires host access to that
   path, so it does not apply to sandboxed/microVM-managed workers, and **macOS
   is not covered** (Claude Code stores credentials in the Keychain there, not a
   file). On boot the worker also does a one-time, read-only import of that file
   into the vault when the vault is present but empty (never written back).

API-key credentials are rejected — they belong on `provider-anthropic` under
provider id `anthropic`.

## Behavior

- **Registration:** self-declares via `router::provider::register` with backoff,
  and re-declares on the `router::ready` trigger. It advertises dynamic model
  listing and `credential_env_var: None` (OAuth-only); identity binds via the
  `registration_token` persisted in state (scope `provider-claude-code`).
- **Models:** attempts the authenticated `GET /v1/models` at startup, on
  explicit refresh, after router readiness, and every ~15 minutes. Picker-visible
  results become **namespaced** router ids (`claude-code/<upstream-id>`). If the
  models endpoint rejects the subscription OAuth token (401/403) — or returns
  nothing — a small **curated fallback slice** is reconciled instead of blanking
  the catalog, so streaming still works. Transient failures preserve the
  router's last-known-good slice. Namespacing prevents `AmbiguousModel`
  collisions with `provider-anthropic`.
- **Request:** Messages API — `messages`, `tools`, `max_tokens`, `stream: true`,
  optional adaptive `thinking` + `output_config.effort`, and automatic prompt
  caching. `system` is always an array whose **first block is the Claude Code
  identity line** (a wire-only artifact required by the subscription backend);
  the router-supplied identity prompt follows as a second block. Headers:
  `authorization: Bearer`, `anthropic-version: 2023-06-01`,
  `anthropic-beta: oauth-2025-04-20`.
- **SSE:** `content_block_delta` (`text_delta` → text, `thinking_delta` →
  thinking, `input_json_delta` → tool calls), `message_delta` → usage,
  `message_stop` → terminal. Unknown event types are ignored (forward-compat).
- **Liveness / errors:** `ping` at least every 30s of silence; 401/403 →
  `auth_expired`, 429 → `rate_limited`, context-overflow → `context_overflow`,
  5xx/network → `transient`, other 4xx → `permanent`. The router owns retry
  policy.

## Running

Standard worker CLI: `--url` (engine WebSocket, default `ws://127.0.0.1:49134`,
or `III_URL`), `--manifest` (print the registry manifest and exit), `--config`
(accepted but ignored — this worker has no file-based config).

```bash
# ensure `claude` has signed in so ~/.claude/.credentials.json exists (dev)
cargo run -- --url ws://127.0.0.1:49134
```

The prompt-cache anchors can be disabled with `PROVIDER_CLAUDE_CODE_CACHE=0`.

## Tests

```bash
cargo test    # unit modules, model-discovery/upstream TCP stubs, schema goldens
```

Regenerate the wire-schema goldens with `UPDATE_GOLDENS=1 cargo test`.

## Troubleshooting

| Symptom | Cause | Fix |
| --- | --- | --- |
| `not configured: sign in with Claude Code …` | no vault credential and no readable `~/.claude/.credentials.json` | run the `oauth-claude-code` sign-in, or `claude` (login) so `~/.claude/.credentials.json` exists; on macOS the Keychain store is not read |
| local fallback is used on each request | `auth-credentials` vault not running | start the vault for shared/refreshing credentials, or keep relying on the local `~/.claude/.credentials.json` fallback |
| `requires a Claude Pro/Max OAuth login … API keys belong on provider-anthropic` | credential is an API key | this provider is OAuth-only; use `provider-anthropic` for keys |
| `auth_expired` on every request | the local `.credentials.json` token expired and no refresh worker is registered | run `claude` once to refresh the file, or register the `oauth-claude-code` refresh flow |
| catalog shows only the curated fallback models | `GET /v1/models` rejected the OAuth bearer, or the models endpoint is unreachable | expected — the subscription token may not be accepted on `/v1/models`; streaming still works, and the live list returns once the endpoint accepts the token |
| upstream 401 despite a valid token | the request no longer resembles Claude Code | keep the identity system block first and the `anthropic-beta: oauth-2025-04-20` header; a future backend change may require a `user-agent` compat header |
| model routes ambiguously | a `claude-code/*` id collided with another provider | keep ids namespaced; or pin `provider: "claude-code"` |
