# Checking whether a credential is configured

## When to use

- A provider adapter wants to short-circuit a request when no credential is configured, instead of getting `null` from `auth::get_token`.
- A health endpoint exposes "auth configured" without revealing the token.
- Diagnostics: confirming which source — stored or environment — is active for a given provider.

## Notes

- The response `source` field is `"stored"` or `"environment"` and is omitted when no credential is configured.
- The optional `label` is a redacted hint (e.g. `"api-key:sk-ant-…"`, `"oauth"`) suitable for display in a UI without leaking the full credential.
- Unlike `auth::get_token`, this function never returns the credential bytes — safe to call from logging or diagnostic paths.
