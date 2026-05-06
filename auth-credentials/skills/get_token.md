# auth::get_token

Read the stored credential for a provider, or null if none is stored.

`({ provider }) → Credential | null` — resolves stored credential first, then
falls back to the process environment (e.g. `ANTHROPIC_API_KEY`). Returns the
full credential object or null.

## When to use

- A provider adapter (e.g. `provider-anthropic`) is about to make an API call and needs the current credential.
- Pre-flight check before issuing a request that requires a credential.

## Notes

- Returns null when the provider has no stored credential and no matching environment variable — distinguish from "credential exists but is empty" via `status`.
- Bus errors surface as `Err`; treat them as transient (engine restart, IPC hiccup) and retry per caller policy.
- The resolution order is: stored credential → environment variable → null. Callers never need to re-read env directly.
