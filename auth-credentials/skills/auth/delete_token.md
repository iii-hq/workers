---
type: how-to
function_id: auth::delete_token
title: Remove a stored provider credential
---

# When to use

Call `auth::delete_token` when the stored credential for a provider should stop being used. The operation is idempotent and only affects the stored backend, not process environment variables.

Reach for it when:

- A user revokes or disconnects a provider credential.
- OAuth refresh has failed repeatedly and the next call should force re-authentication.
- A test wrote a temporary credential and needs to clean it up.

Use [`auth::status`](iii://auth-credentials/auth/status) after deletion when you need to know whether an environment fallback still keeps the provider configured.

# Inputs

```json
{
  "provider": "anthropic"               // required, non-empty provider id
}
```

`provider` identifies the stored credential record to remove.

# Outputs

```json
{
  "ok": true                            // true even when no stored record existed
}
```

- Deleting a missing stored credential is not an error.
- Environment fallbacks are not removed; unset the environment variable outside this worker if fallback should disappear too.

# Side effects

Deletes one stored backend record:

```json
{
  "scope": "auth_credentials",
  "key": "credential:anthropic"
}
```

With the `memory` backend, the provider entry is removed from the current process map.

# Worked example

Remove a stored Anthropic credential:

```json
{
  "provider": "anthropic"
}
```

# Related

- `auth::get_token` — confirm whether an environment fallback still resolves.
- `auth::status` — check the active credential source after deletion.
- `auth::set_token` — store a replacement credential.
