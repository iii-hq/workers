---
type: how-to
function_id: auth::get_token
title: Read a provider credential
---

# When to use

Call `auth::get_token` when a provider adapter is about to make an API request and needs the credential it should send to the external provider. Resolution checks stored credentials first, then the matching process environment variable.

Reach for it when:

- `provider-anthropic`, `provider-openai`, or another adapter needs credentials immediately before an API call.
- A worker wants the same stored-then-environment fallback behavior as every other provider adapter.
- A setup still relies on variables such as `ANTHROPIC_API_KEY` or `OPENAI_API_KEY`.

Use [`auth::status`](iii://auth-credentials/auth/status) instead for health checks, diagnostics, or any path that may log the response.

# Inputs

```json
{
  "provider": "anthropic"               // required, non-empty provider id
}
```

`provider` is matched against stored records first. If no stored record exists, known provider ids fall back to their mapped environment variable.

# Outputs

```json
{
  "type": "api_key",                    // "api_key" or "oauth"
  "key": "sk-ant-..."                   // present for type "api_key"
}
```

- Returns `null` when neither a stored credential nor an environment fallback exists.
- Stored credentials have precedence over environment variables.
- OAuth credentials return `access_token`, optional `refresh_token`, optional `expires_at`, `scopes`, and `provider_extra`.

# Worked example

Read the Anthropic credential before calling the provider:

```json
{
  "provider": "anthropic"
}
```

If `auth::set_token` stored a credential for `anthropic`, that credential is returned. Otherwise, the worker checks `ANTHROPIC_API_KEY` and returns an `api_key` credential when the variable is present and non-empty.

# Related

- `auth::set_token` — store or rotate the credential this function reads.
- `auth::status` — inspect whether resolution will succeed without returning the secret.
- `auth::delete_token` — remove the stored credential so environment fallback can take over.
