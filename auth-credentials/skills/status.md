# auth-credentials/status

Check whether a credential is stored for a provider, and its configuration details.

`auth::status({ provider }) → { configured, source?, label? }` — `configured`
is a boolean; `source` is one of `"stored"`, `"environment"` (omitted when not
configured); `label` is a short human-readable hint (e.g. `"api-key:sk-ant-…"`,
`"oauth"`) useful for display without revealing the full credential.

## When to use

- A provider adapter wants to short-circuit a request when no credential is configured (instead of getting null from `get_token`).
- A health endpoint exposes "auth configured" without revealing the token.
- Diagnostics: confirming which source (stored vs. env) is active for a given provider.
