# provider-openai-codex

OpenAI **Codex (ChatGPT subscription)** provider worker behind
[llm-router](https://github.com/iii-hq/workers/tree/main/llm-router). It lets the stack generate against a ChatGPT/Codex
subscription (billed to the plan, "Sign in with ChatGPT") instead of a
pay-per-token API key, by speaking OpenAI's **Responses API** at the Codex
backend.

Implements the provider protocol from `tech-specs/2026-06-agentic/llm-router.md`:
`provider::openai-codex::stream` (Responses SSE → `AssistantMessageEvent` frames
into a router-owned channel), `provider::openai-codex::refresh_models`
(fetches and reconciles the authenticated Codex model catalog), and
`provider::openai-codex::count_tokens` (local prompt token estimation with
the tiktoken tokenizers behind `router::count_tokens`; never runs the model,
costs nothing, and needs no network).

> ⚠️ **Terms-of-service caveat — local/personal dev only.** This drives a
> personal ChatGPT subscription through the undocumented
> `https://chatgpt.com/backend-api/codex` backend with a Codex-client
> `originator` header. That is plausibly against OpenAI's terms and may risk
> account action; the backend is unversioned and can change without notice. Use
> it for local, personal development. For team/CI/production, use official
> API-key billing (`provider-openai`). You assume the risk.

## Credentials

Open this provider's configuration form in the console and select **Sign in
with ChatGPT**. Open the verification link, enter the displayed code, and
approve the login. The provider finishes the login in the background, saves the
session, and refreshes the model catalog. No Codex CLI is required, including
when the provider runs on a remote machine.

Device login must be enabled in your ChatGPT security settings or workspace
permissions. Codes expire after 15 minutes. Cancel or start a new attempt if
needed; reopening the form resumes an attempt while the worker is running.
Restarting the worker cancels pending attempts, but retains completed logins.

The form shows session status independently from the saved model catalog. Use
**Refresh models** to retry catalog discovery, **Switch account** to replace
the active account after a successful login, or **Log out** to sign out this
provider. Logging out does not sign out the Codex CLI or revoke other clients.
The provider will not silently reconnect using a local or external credential.

One account is shared by this provider's consumers in each engine namespace.
Run one provider instance per namespace. Its tokens are kept in the private
`provider-openai-codex-auth` state scope, separate from router registration.
The state worker's public API and state-change subscriptions cannot expose that
scope. Keep the state adapter durable to retain sessions across stack restarts;
a private namespace provides access separation, not encryption at rest.

Managed credentials renew automatically within 60 seconds of expiry. Refresh
requests are serialized and token rotations are persisted atomically. A
revoked session requires another login; temporary storage/network errors do
not erase it or switch accounts. The provider never returns tokens to the UI
or writes them into router configuration.

Before the first managed login or explicit disconnect, existing installations
can still use `auth::get_token` from an external credential vault, then a
read-only `${CODEX_HOME:-$HOME/.codex}/auth.json` fallback. The vault remains
responsible for its own refresh. The provider never imports or writes the CLI
file, and never refreshes the CLI's tokens. The file fallback needs host access
and does not support an OS keyring-only login.

### Authentication functions

All IDs below begin with `provider::openai-codex::`. These are operator APIs;
the harness, event bindings, and guarded dispatchers deny agent calls to them
and to the provider's private state accessors. These checks rely on the existing
trusted-engine boundary: they do not isolate credentials from arbitrary code
with a direct engine connection or host filesystem access.

| Function | Input | Result |
| --- | --- | --- |
| `login::start` | `{}` | `login_id`, `verification_uri`, `user_code`, `expires_at` (Unix seconds), `interval` (seconds) |
| `login::poll` | `{login_id}` | `status`: `pending`, `ok`, `expired`, `canceled`, or `error`; sanitized error when present |
| `login::cancel` | `{login_id}` | `{ok: true}`; idempotent, keeps the active account |
| `auth::status` | `{}` | `status`: `signed_out`, `authenticated`, or `expired`; `source`, `account_id`, pending `login` |
| `auth::logout` | `{}` | `{ok: true}` after disconnect is persisted |

Unknown or pre-restart login IDs return `canceled`. Storage failures return an
error instead of a success acknowledgment. Tokens stay entirely in the backend.

API-key credentials are rejected — they belong on `provider-openai` under
provider id `openai`.

## Behavior

- **Registration:** self-declares via `router::provider::register` with backoff,
  and re-declares on the `router::ready` trigger. It advertises dynamic model
  listing and `credential_env_var: None`; identity binds via the
  `registration_token` persisted in state (scope `provider-openai-codex`).
- **Models:** fetches the account-scoped Codex catalog from authenticated
  `GET /backend-api/codex/models?client_version=…` at startup, on explicit
  refresh, after successful sign-in, after router readiness, and every three minutes. Picker-visible
  results become **namespaced** router ids (`codex/<upstream-id>`). Each
  successful non-empty response replaces the complete provider slice, adding
  new models and removing retired ones. Failed or empty refreshes preserve the
  router's persisted last-known-good slice. Namespacing prevents
  `AmbiguousModel` collisions with `provider-openai`.
- **Request:** Responses API — `input` items, `stream: true`, `store: false`,
  optional `tools` and `reasoning: { effort }`. Headers: `Authorization: Bearer`,
  `chatgpt-account-id`, Codex compatibility `version`,
  `openai-beta: responses=experimental`,
  `originator: codex_cli_rs`.
- **Cache routing:** the `session-id` / `thread-id` / `x-client-request-id`
  headers always carry a UUID derived from the router's `session_id`. The body
  `prompt_cache_key` is resolved separately: a caller's
  `provider_options.openai-codex.prompt_cache_key` override, else a UUID
  derived from the router's `cache_intent.surface_digest` (the frozen
  agent-profile prefix), else the same session-derived UUID.

  The subscription backend caches per session, not per content: a second turn
  in the same session reads the prefix from cache (`cached_tokens` ~17.9k on an
  18k-token prefix), but an independent session on a byte-identical prefix
  reads 0 and the backend records `cache_write_tokens: 0` for it. The cache
  entry is keyed to the session-affinity headers, which are per-session by
  design, so a shared body `prompt_cache_key` does not move routing to a shared
  shard, and cross-session reuse is not achievable here without reusing session
  identity across independent agents. The backend also rejects
  `prompt_cache_breakpoint` on every model (`gpt-6-astra` included:
  `prompt_cache_breakpoint is not supported on this model`), so the explicit
  breakpoint that `provider-openai` uses on GPT-5.6+ does not apply. Read
  `usage.cache_read` for the real signal; the shared body key stays in case the
  backend's routing changes.
- **SSE:** `response.output_text.delta` → text, `response.reasoning_*` →
  thinking, `response.function_call_arguments.delta` → tool calls,
  `response.completed` → usage + terminal. Unknown event types are ignored
  (forward-compat).
- **Liveness / errors:** `ping` at least every 30s of silence; 401/403 →
  `auth_expired`, 429 → `rate_limited`, `context_length_exceeded` →
  `context_overflow`, 5xx/network → `transient`, other 4xx → `permanent`. The
  provider attempts one managed-token refresh and retry on HTTP 401 before
  streaming starts; it never replays content already delivered. The router
  owns other retry policy.

## Image generation

`provider::openai-codex::image::generate` and
`provider::openai-codex::image::read` are the **agent-callable** functions of
this worker (images have no router front door). The Codex backend has no
`/images/generations` endpoint, so the picture is made the way the Codex CLI
makes it: a Responses call on a Codex chat model (`host_model`, default
`gpt-5.5`; `codex/…` router ids accepted) carries the hosted
`image_generation` tool pinned to the requested image model, `tool_choice`
forces that tool, and the streamed `image_generation_call` item's `result`
is the image. Billing follows the ChatGPT plan.

`generate` writes the picture, at full resolution, to
`data/provider-openai-codex/images/<stem>-<unix_ms>-<8 hex>.<ext>` (relative
to the Compose project directory, `III_COMPOSE_DIR`) and returns that path
with **no image bytes** — base64 in a function result only inflates the
conversation context.

```jsonc
// request
{ "model": "gpt-image-2.5-flare",         // or gpt-image-2.5-sunburst (anything else is rejected)
  "prompt": "a lighthouse at dusk, oil painting",
  "size": "1024x1536",                     // 1024x1024 | 1536x1024 | 1024x1536 | auto
  "quality": "medium",                     // low | medium | high | auto
  "output_format": "png",                  // png (default) | jpeg | webp
  "background": "auto",                    // transparent | opaque | auto
  "file_name": "lighthouse",               // optional stem for the saved file
  "host_model": "gpt-5.5" }                // optional Codex chat model hosting the tool call

// response — the same envelope as browser::screenshot, minus the picture
{ "content": [
    { "type": "text", "text": "Image generated by gpt-image-2.5-flare via Codex (1024x1536, 1612044 bytes, image/png) saved to /…/data/provider-openai-codex/images/lighthouse-1790171000000-8c21be07.png; view with provider::openai-codex::image::read { path }" } ],
  "details": { "model", "host_model", "path", "mime", "bytes", "size", "quality", "width", "height", "inline": "none", "revised_prompt?", "usage?" } }
```

**Preview in the chat.** The injected console UI (`ui/src/image-renderer.tsx`)
renders the card for a `generate` call by fetching a downscaled JPEG from
this worker through `image::read`, and shows it beside the copyable path,
dimensions, size and models — the bytes travel worker → console, never
through the model.

**Looking at the picture from an agent.** `image::read { path, variant? }`
takes the `details.path` a generate call returned (or the bare file name)
and returns viewable content blocks: `variant: "preview"` (default; JPEG
≤512px / ~80 KB, fits the harness's 256 KiB result cap) or `variant:
"full"`. Anything outside `data/provider-openai-codex/images/` is refused,
symlinks included. `generate` also accepts `inline: "preview" | "full"` to
embed the picture in its own result. If the host model answers in prose
instead of calling the tool (a plan without image generation, a refused
prompt), the error carries its text so the reason is visible.

## Running

Standard worker CLI: `--url` (engine WebSocket, default `ws://127.0.0.1:49134`,
or `III_URL`), `--manifest` (print the registry manifest and exit), `--config`
(accepted but ignored — this worker has no file-based config).

```bash
cargo run -- --url ws://127.0.0.1:49134
```

## Tests

```bash
cargo test    # OAuth/session modules, HTTP/SSE stubs, schema goldens
pnpm --dir ui test
pnpm --dir ui build
```

Regenerate the wire-schema goldens with `UPDATE_GOLDENS=1 cargo test`.

## Troubleshooting

| Symptom | Cause | Fix |
| --- | --- | --- |
| `not configured: sign in with ChatGPT …` | no active session | select **Sign in with ChatGPT** in the provider form |
| `device_login_disabled` | device-code sign-in is disabled | enable it in ChatGPT security settings or ask your workspace administrator |
| `storage_unavailable` | state worker is down or lacks private namespace support | start/update the state worker and retry; credentials are not replaced by a fallback |
| `auth_expired` | token was revoked or the CLI fallback expired | sign in again in the provider form |
| `requires a ChatGPT OAuth login … API keys belong on provider-openai` | credential is an API key | this provider is OAuth-only; use `provider-openai` for keys |
| `missing ChatGPT account id` | token lacks the account claim | sign in again with a ChatGPT account |
| backend `Unsupported parameter` / shape errors | Codex backend contract drifted | update this worker's request/SSE mapping against the current backend |
| model refresh fails or returns no visible models | auth/network/backend catalog problem | the last known catalog is retained; fix the underlying error and call `provider::openai-codex::refresh_models` |
| model routes ambiguously | a `codex/*` id collided with another provider | keep codex ids namespaced; or pin `provider: "openai-codex"` |

The OAuth adapter follows the upstream [Codex device login](https://github.com/openai/codex/blob/rust-v0.153.4/codex-rs/login/src/device_code_auth.rs)
and token refresh protocol at the provider's current compatibility version.
See [OpenAI authentication guidance](https://learn.chatgpt.com/docs/auth) for
account/workspace setup.
