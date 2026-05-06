# auth::delete_token

Remove the stored credential for a provider.

`({ provider }) → { ok: true }` — idempotent. Deleting a provider that has
no stored credential is not an error.

## When to use

- A user revokes their API key.
- An OAuth refresh is failing repeatedly and the credential should be cleared so the next call forces re-auth.
- Cleaning up after a test run that wrote a temporary credential.
