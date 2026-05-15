---
type: how-to
function_id: auth::list_providers
title: List providers with stored credentials
---

# When to use

Call `auth::list_providers` when a caller needs to enumerate which providers have credentials stored in the auth backend without exposing credential bytes.

Reach for it when:

- Building a settings surface that shows connected providers.
- Auditing stored credentials without reading secrets.
- Deciding which provider-specific status checks to run next.

Use [`auth::status`](iii://auth-credentials/auth/status) instead when environment-variable fallback providers must be included.

# Inputs

```json
{}
```

The function accepts an empty object. Any provider filtering should happen client-side after this call.

# Outputs

```json
{
  "providers": [
    "anthropic",
    "openai"
  ]
}
```

- Returns stored provider names only; environment-variable fallbacks do not appear.
- Provider names are sorted lexicographically and duplicate names are removed.
- Token bytes are never included in the response.

# Worked example

List providers with stored credentials:

```json
{}
```

# Related

- `auth::status` — include stored and environment-backed configuration for one provider.
- `auth::get_token` — read the credential for a provider after selecting it.
- `auth::delete_token` — remove a provider returned by this list.
