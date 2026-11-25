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
into a router-owned channel), `provider::claude-code::refresh_models` (fetches
and reconciles the model catalog, with a curated fallback), and
`provider::claude-code::count_tokens` (exact prompt token counts behind
`router::count_tokens`). For **API-key**
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

This worker is a **dumb token consumer** — login and refresh live out-of-band.
Every call gets its token through one account source (`src/accounts.rs`), which
decides between two paths:

### Accounts held by the tmux worker

When the [`tmux`](../tmux/) worker registers `tmux::accounts::credential`, it
holds the Claude Code accounts, and every token comes from it:

- **Which account:** a stream uses `provider_options.account`, the concrete
  account id the harness froze on the turn. Without it (an older harness, a
  direct `router::chat`), and for model discovery and token counting, which
  have no turn, the provider uses the **global account**: it reads the id from
  `tmux::accounts::list`'s `global` field (kept 30 s, and named again after a
  401 or an `unknown_account`) and asks for that id, so the token cache never
  serves the previous account after a switch. A `global` of `null` (no
  account signed in as the global one) is `auth_expired`. tmux's answer must
  name the account asked for when it names one; a token for another account
  is never used.
- **A pinned account never falls back:** a stream with
  `provider_options.account` runs on that account's token or not at all.
  Without the tmux worker, or while it cannot be told whether tmux holds the
  accounts, it fails as `transient` instead of running on the `~/.claude`
  token, which is another account (and its quota).
- **Detection:** "tmux manages the accounts" means
  `engine::functions::list {prefix: "tmux::accounts::", include_internal: true}`
  lists `tmux::accounts::credential`. The answer is kept 30 s. A list call that
  fails is not absence: it is tried three times (250 ms, then 500 ms apart);
  then the last answer stands, and with none known the call fails as
  `transient` (it never takes the vault path, and the boot import below runs
  only on a confirmed absence). A `function_not_found` from tmux drops the
  answer so the next call looks again. The function is the signal rather than tmux's
  account registry (`claude.managed` in its usage): tmux serves and renews the
  global account's token even without a registry, so a vault copy of that
  token would drift from the one tmux renews.
- **No vault:** while tmux manages the accounts, the provider neither reads nor
  writes the `auth-credentials` vault, and the boot import below is skipped.
- **Cache:** each account's token is cached by id until 60 s before its
  `expires_at_ms`; one without an expiry is never cached, and an expired one is
  never sent. An answer that arrives after the account's token was dropped
  (a 401 elsewhere) is used once and not cached. tmux renews a token that
  expires within five minutes.
- **A 401:** the provider drops that account's token, asks tmux once more and,
  when it gets a different token, sends the request again once. A 401 that
  survives is `auth_expired` ("Claude Code rejected the sign-in of the account
  …"). When tmux gives no token on that second ask, its own refusal stands
  (below): `busy` stays `transient`, not a sign-in to redo. Streaming,
  discovery and token counting all do this.
- **tmux's refusals** become the turn's error frame: `not_signed_in` and
  `token_expired` → `auth_expired`, `busy` (a renewal under way) or tmux not
  answering → `transient` (the router retries), `unknown_account` →
  `permanent`. Discovery keeps the last slice on all of them, and fails with
  `auth_expired` when the global account must sign in again.

### Without the tmux worker

Today's path, unchanged, for every call on the global account (a stream
without `provider_options.account`, model discovery, token counting):

1. **Vault (intended authority):** the `auth-credentials`
   worker (`auth::get_token`), populated by an `oauth-claude-code` sign-in flow.
   A near-expiry token makes this provider call the vault-owned refresh
   (`oauth::claude-code::refresh`); it never calls the OAuth token endpoint
   itself. That worker/flow is out of scope here — this provider only
   *consumes* and *calls*. The credential record it expects is
   `{ type: "oauth", access_token, refresh_token?, expires_at?(seconds),
   provider_extra: { subscription_type?, scopes? }, refresh_fn:
   "oauth::claude-code::refresh" }`.
2. **Local dev fallback:** when no vault is running, the worker reads the
   `claude` CLI's own credential (read-only — the CLI owns its refresh,
   including rotating the refresh token): `${CLAUDE_CONFIG_DIR:-$HOME/.claude}/.credentials.json`
   and, on macOS, the "Claude Code-credentials" login Keychain item (via
   `security find-generic-password`), taking whichever expires later — the
   macOS CLI stops updating the file once it uses the Keychain. The
   `claudeAiOauth.expiresAt` is epoch **milliseconds**; it is stored as seconds
   in the vault shape. Requires host access to those stores, so it does not
   apply to sandboxed/microVM-managed workers. On boot the worker also does a
   one-time, read-only import of that credential into the vault when the vault
   is present but empty (never written back).

API-key credentials are rejected — they belong on `provider-anthropic` under
provider id `anthropic`.

## Behavior

- **Registration:** self-declares via `router::provider::register` with backoff,
  and re-declares on every `router::on-ready` event (it observes that function
  at boot). It advertises dynamic model
  listing and `credential_env_var: None` (OAuth-only); identity binds via the
  `registration_token` persisted in state (scope `provider-claude-code`).
- **Models:** attempts the authenticated `GET /v1/models` at startup, on
  explicit refresh, after router readiness, and every ~15 minutes. Picker-visible
  results become **namespaced** router ids (`claude-code/<upstream-id>`). If the
  models endpoint refuses the subscription OAuth token (403) — or returns
  nothing — a small **curated fallback slice** is reconciled instead of blanking
  the catalog, so streaming still works. A 401 (the token itself was rejected)
  or an expired token fails the refresh with `auth_expired`, so setup asks for
  a new sign-in instead of listing models a chat cannot use. Those failures and
  transient ones preserve the router's last-known-good slice. Namespacing prevents `AmbiguousModel`
  collisions with `provider-anthropic`.
- **Request:** Messages API — `messages`, `tools`, `max_tokens`, `stream: true`,
  optional adaptive `thinking` + `output_config.effort`, and automatic prompt
  caching. `system` is always an array whose **first block is the Claude Code
  identity line** (a wire-only artifact required by the subscription backend);
  an optional router-supplied harness prompt follows as a second block. Headers:
  `authorization: Bearer`, `anthropic-version: 2023-06-01`,
  `anthropic-beta: oauth-2025-04-20`.
- **Token counting:** `provider::claude-code::count_tokens` (behind
  `router::count_tokens`) posts the stream body (identity block, the
  `thinking` / `output_config.effort` mapped from an optional `thinking_level`,
  the trailing-assistant drop; no `max_tokens`, `stream` or cache markers) to
  the messages endpoint's
  `count_tokens` sibling (`/v1/messages/count_tokens`) with the same OAuth
  credential and headers as streaming, for an exact provider-metered count
  (`estimator: "provider"`). It never runs the model and costs nothing; a
  non-2xx answer is returned as an error so the router can fall back to an
  estimate.
- **SSE:** `content_block_delta` (`text_delta` → text, `thinking_delta` →
  thinking, `input_json_delta` → tool calls), `message_delta` → usage,
  `message_stop` → terminal. Unknown event types are ignored (forward-compat).
- **Liveness / errors:** `ping` at least every 30s of silence; 401/403 →
  `auth_expired`, 429 → `rate_limited`, context-overflow → `context_overflow`,
  5xx/network → `transient`, other 4xx → `permanent`. The router owns retry
  policy.
- **Usage limit:** a 429 whose `anthropic-ratelimit-unified-status` is
  `rejected` means the account's subscription limit is reached, not a busy
  moment (`limits::is_usage_limit`). Its error frame stays `rate_limited` and
  carries `error_details: {usage_limit: true, window?, resets_at_ms?}`: the
  window from `anthropic-ratelimit-unified-representative-claim` (`five_hour`
  → `five_hour`, `seven_day*` → `week`, anything else left out) and the reset
  from `anthropic-ratelimit-unified-reset` (else the window's own
  `-5h-reset` / `-7d-reset`), unix seconds turned into milliseconds. The
  router never retries it and hands the details to the caller. These are the
  headers the Claude Code CLI reads; Anthropic's public API docs do not list
  the `unified` family.

## Running

Standard worker CLI: `--url` (engine WebSocket, default `ws://127.0.0.1:49134`,
or `III_URL`), `--manifest` (print the registry manifest and exit), `--config`
(accepted but ignored — this worker has no file-based config).

```bash
# ensure `claude` has signed in (dev): ~/.claude/.credentials.json or, on macOS, the Keychain
cargo run -- --url ws://127.0.0.1:49134
```

The prompt-cache anchors can be disabled with `PROVIDER_CLAUDE_CODE_CACHE=0`.
When the router forwards `system_sections`, the identity block is followed by
one text block per section and the marker lands on the block flagged
`cache_boundary` (the frozen agent-profile prefix) instead of the last block,
whatever its size (Anthropic applies its per-model token minimum over the
whole prefix and silently skips a short one), so the per-session tail no
longer invalidates the shared entry. That boundary
block and the tools marker use the 1-hour cache (`ttl: "1h"`); the messages
anchor stays at 5 minutes unless `PROVIDER_CLAUDE_CODE_CACHE_MESSAGES_TTL=1h`
asks for one hour there too (only on top of the 1-hour prefix), for agents that
sleep between turns, such as an orchestrator waiting on its children: one that
wakes after more than five minutes otherwise writes its whole conversation
again. `PROVIDER_CLAUDE_CODE_CACHE_TTL=5m` restores the 5-minute cache
everywhere.

## Tests

```bash
cargo test    # unit modules, model-discovery/upstream TCP stubs, schema goldens
```

Regenerate the wire-schema goldens with `UPDATE_GOLDENS=1 cargo test`.

## Troubleshooting

| Symptom | Cause | Fix |
| --- | --- | --- |
| `not configured: sign in with Claude Code …` | no vault credential and no local `claude` credential (`~/.claude/.credentials.json`, or the Keychain on macOS) | run the `oauth-claude-code` sign-in, or `claude` (login) |
| local fallback is used on each request | `auth-credentials` vault not running | start the vault for shared/refreshing credentials, or keep relying on the local `claude` credential fallback |
| `requires a Claude Pro/Max OAuth login … API keys belong on provider-anthropic` | credential is an API key | this provider is OAuth-only; use `provider-anthropic` for keys |
| `Your Claude Code sign-in has expired …` | the local `claude` token expired and no refresh worker is registered | run `claude` once to refresh it, or register the `oauth-claude-code` refresh flow |
| `The Claude Code account "…" is not signed in …` / `… has expired and could not be renewed …` | the tmux worker holds the accounts and that one has no usable sign-in | sign in to that account again through the tmux worker |
| `The Claude Code account "…" is busy renewing its sign-in …` | tmux was renewing that account's token | nothing: the router retries; send again if it gave up |
| `This Claude Code account has reached its usage limit.` | the account's five-hour or weekly subscription limit (`error_details.usage_limit`) | wait for `resets_at_ms`, or switch the turn to another account |
| `The Claude Code accounts could not be read (the tmux worker that holds the account "…" is not running) …` | a turn pinned to an account, with the tmux worker stopped (or its function list unreadable) | start the tmux worker; the router retries meanwhile |
| the vault is never consulted | the tmux worker registers `tmux::accounts::credential`, so it holds the accounts | expected while the tmux worker runs; without it the provider is back on the vault / `~/.claude` path within 30 s |
| `Claude Code rejected the current sign-in …` | Anthropic answered 401: the token was revoked, replaced, or is corrupted | run `claude` and `/login` again |
| `Anthropic refused this request for the current Claude Code sign-in …` | Anthropic answered 403: the token is valid but not allowed this request | signing in again does not help; check the reason in parentheses, such as the plan's access to the model |
| catalog shows only the curated fallback models | `GET /v1/models` refused the OAuth bearer (403), or returned nothing | expected — streaming still works, and the live list returns once the endpoint accepts the token |
| upstream 401 despite a valid token | the request no longer resembles Claude Code | keep the identity system block first and the `anthropic-beta: oauth-2025-04-20` header; a future backend change may require a `user-agent` compat header |
| model routes ambiguously | a `claude-code/*` id collided with another provider | keep ids namespaced; or pin `provider: "claude-code"` |
