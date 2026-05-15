---
type: index
title: auth-credentials
---

# auth-credentials

Provider credential vault on the iii bus. Use it to store, read, inspect, and revoke provider API keys or OAuth tokens through `auth::*` so provider adapters and agents do not read raw secrets from their own configuration paths.

- **Credentials** (`auth::*`) — durable credential reads and writes for providers such as `anthropic`, `openai`, and `google`.

Prefer `auth::status` over `auth::get_token` for pre-flight gating. `auth::status` returns no token bytes, so it is safe to log, and the `source` field distinguishes a stored credential from an environment-variable fallback.

## How-tos

### `auth::*`

- [`auth::set_token`](iii://auth-credentials/auth/set_token) — store or rotate an API key or OAuth credential.
- [`auth::get_token`](iii://auth-credentials/auth/get_token) — read the credential a provider adapter should use for an API call.
- [`auth::delete_token`](iii://auth-credentials/auth/delete_token) — remove a stored credential without touching environment fallbacks.
- [`auth::list_providers`](iii://auth-credentials/auth/list_providers) — list provider names that have stored credentials.
- [`auth::status`](iii://auth-credentials/auth/status) — check whether a provider is configured without returning secret bytes.
