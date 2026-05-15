---
type: how-to
function_id: auth::set_token
title: Store a provider credential
---

# When to use

Call `auth::set_token` when a provider credential should become the stored credential of record for later `auth::*` reads. The write replaces any existing stored credential for the same provider.

Reach for it when:

- A user supplies an API key during setup.
- An OAuth worker returns a fresh access token that downstream provider adapters need to read.
- A credential is rotated after a security incident.

Use [`auth::status`](iii://auth-credentials/auth/status) instead when you only need to know whether a provider is configured.

# Inputs

```json
{
  "provider": "anthropic",              // required, non-empty provider id
  "credential": {
    "type": "api_key",                  // required, "api_key" or "oauth"
    "key": "sk-ant-..."                 // required for type "api_key"
  }
}
```

For OAuth credentials, `credential` uses `type: "oauth"` with `access_token`, optional `refresh_token`, optional `expires_at`, optional `scopes`, and optional `provider_extra`.

# Outputs

```json
{
  "ok": true                            // true when the credential was stored
}
```

- Validation fails when `provider` is empty or only whitespace.
- Writes overwrite the whole stored credential for the provider; there is no merge.

# Side effects

Persists one credential record in the configured backend:

```json
{
  "provider": "anthropic",
  "credential": {
    "type": "api_key",
    "key": "sk-ant-..."
  }
}
```

With the default `iii_state` backend, the record is written under scope `auth_credentials` and key `credential:<provider>`. With the `memory` backend, the record only lives for the current worker process.

# Worked example

Store an Anthropic API key:

```json
{
  "provider": "anthropic",
  "credential": {
    "type": "api_key",
    "key": "sk-ant-redacted"
  }
}
```

Store an OAuth credential:

```json
{
  "provider": "anthropic",
  "credential": {
    "type": "oauth",
    "access_token": "access-token-redacted",
    "refresh_token": "refresh-token-redacted",
    "expires_at": 1893456000,
    "scopes": ["messages:write"],
    "provider_extra": {
      "tenant": "workspace-a"
    }
  }
}
```

# Related

- `auth::get_token` — read the credential after it is stored.
- `auth::status` — verify configuration without exposing token bytes.
- `auth::delete_token` — remove a stored credential.
