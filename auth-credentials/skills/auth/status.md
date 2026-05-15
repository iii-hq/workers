---
type: how-to
function_id: auth::status
title: Check provider credential status
---

# When to use

Call `auth::status` when a caller needs to know whether a provider is configured without returning the credential itself. It uses the same stored-then-environment resolution order as `auth::get_token`.

Reach for it when:

- A provider adapter wants to short-circuit before making an API request.
- A health endpoint needs to expose "auth configured" safely.
- Diagnostics need to distinguish stored credentials from environment fallbacks.

Use [`auth::get_token`](iii://auth-credentials/auth/get_token) instead only on the execution path that will immediately use the credential.

# Inputs

```json
{
  "provider": "anthropic"               // required, non-empty provider id
}
```

`provider` is checked against stored records first, then against the known environment-variable map.

# Outputs

```json
{
  "configured": true,                   // true when stored or environment credential exists
  "source": "stored",                   // "stored" or "environment"; omitted when unconfigured
  "label": "api-key:sk-ant…"            // redacted display hint; omitted when unconfigured
}
```

- `source` is omitted when `configured` is false.
- `label` never contains the full credential. API key labels include only the first six characters.
- OAuth credentials use the label `"oauth"`.

# Worked example

Check whether Anthropic auth is configured:

```json
{
  "provider": "anthropic"
}
```

# Related

- `auth::get_token` — read the credential only when a provider call needs it.
- `auth::set_token` — store a credential when status is unconfigured.
- `auth::delete_token` — remove the stored source reported by status.
