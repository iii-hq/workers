# auth-credentials

Provider credential vault. Stores API keys and OAuth tokens under `auth::*` so providers and other workers never see raw secrets.

## When to use

- An agent or provider needs an API key for an LLM, search, or storage backend, and you want to avoid hardcoding.
- A user just authenticated via an OAuth flow (e.g. `oauth-anthropic`) and the resulting token needs to be persisted for later resolution.
- A workflow needs to look up the active credential for a provider without knowing whether it came from an env var, the keychain, or an OAuth refresh.

## Functions

- `auth::set_token(provider, token)` — store an API key or OAuth token for a provider.
- `auth::get_token(provider) → token?` — read the stored token for a provider, or null.
- `auth::list_providers() → [provider]` — list providers with stored tokens.
- `auth::delete_token(provider)` — remove a stored token.
- `auth::status(provider) → { has_token, kind }` — check whether a credential is stored and what shape it has.

## When NOT to use

- For workspace API keys and roles (HMAC tokens, owner/admin/member/viewer): use `auth-rbac`. The two share no state.
- For one-off OAuth flows (browser handoff, PKCE, device code): use the matching `oauth-*` worker; it then writes the result here.

## Notes

- Default backend is `iii_state` (durable). Set `AUTH_CREDENTIALS_STORE=memory` for ephemeral in-process storage in tests.
- All five functions are local state operations (no network calls).
- Transient bus errors surface as `Err` to the caller; retry policy is the caller's choice.
