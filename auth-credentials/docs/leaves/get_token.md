# Reading a provider credential before an API call

## When to use

- A provider adapter (e.g. `provider-anthropic`) is about to make an upstream call and needs the current credential.
- Pre-flight resolution before issuing a request that requires a token.
- One-off debugging where a stored credential needs to be inspected directly.

## Notes

- Resolution order is: stored credential → matching environment variable → null. Callers never need to read the environment directly.
- A `null` return means neither a stored credential nor an env fallback exists — distinguish from "credential exists but is empty" via `auth::status`.
- Bus errors surface as `Err`; treat them as transient (engine restart, IPC hiccup) and retry per caller policy.
