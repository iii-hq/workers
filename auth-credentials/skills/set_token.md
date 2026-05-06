# auth-credentials/set_token

Store a credential for a provider so later `get_token` and `status` calls
can resolve it.

`auth::set_token({ provider, credential }) → { ok: true }` — overwrites any
existing credential for that provider. `credential` is a typed object:
`{ type: "api_key", key }` or `{ type: "oauth", access_token, ... }`.
No verification at write time.

## When to use

- A user just supplied an API key during setup.
- An OAuth flow (e.g. `oauth-anthropic`) just returned a fresh access token.
- Rotating a credential after a security incident.

## Notes

- Default backend is `iii_state` (durable). Set `AUTH_CREDENTIALS_STORE=memory` for ephemeral storage in tests.
- Caller validates token shape; the worker only stores the serialised credential bytes.
