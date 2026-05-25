# auth-credentials

Multi-provider credential store on the iii bus (`auth::*`).

## Purpose

Centralised storage for provider credentials -- API keys and OAuth tokens
-- used by the provider workers (`provider-anthropic`, `provider-openai`,
etc.) to authenticate outbound calls. Persistence is delegated to the
[`database`](https://github.com/iii-ai/iii/tree/main/workers/database)
worker; a single SQLite file (default `~/.iii/harness.db`) backs both
`auth-credentials` and its sibling `provider-config`.

`auth::get_token` is the resolver every consumer calls. It first looks up
a stored credential; if none, it falls back to the per-provider
environment variable defined in
[src/auth-credentials/types.ts](harness/src/auth-credentials/types.ts)
(`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `AWS_ACCESS_KEY_ID`, `HF_TOKEN`,
etc.) and synthesises an `ApiKey` credential. `auth::status` reports
`{ configured, source: "stored" | "environment", label }` without
returning the secret material itself.

[iii-permissions.yaml](iii-permissions.yaml) denies `auth::set_token` and
`auth::delete_token` to in-run agents (kernel rules). `auth::get_token`,
`auth::list_providers`, and `auth::status` are on the bare-string allow
list.

## Registered functions

- `auth::get_token` -- Fetch the stored credential for a provider.
- `auth::set_token` -- Persist a credential for a provider.
- `auth::delete_token` -- Remove the stored credential for a provider.
- `auth::list_providers` -- List every provider with a stored credential.
- `auth::status` -- Report stored vs. env credential status for a provider.

## Triggers

None.

## On-disk layout

Backed by `database::query` / `database::execute` against the
`auth_credentials` table:

| Table | Columns | Purpose |
|---|---|---|
| `auth_credentials` | `provider TEXT PRIMARY KEY, payload TEXT NOT NULL, updated_at INTEGER NOT NULL` | One row per provider; `payload` is a `Credential` value serialized as JSON -- either `{ type: "api_key", key }` or `{ type: "oauth", access_token, refresh_token?, expires_at?, scopes?, provider_extra? }`. |

Mutations use `INSERT ... ON CONFLICT(provider) DO UPDATE`, so each
`set` / `delete` is atomic at the SQL layer.

## Configuration

Shared pool name resolution is documented in
[storage.md](../storage.md). From [config.yaml](harness/config.yaml):

```yaml
storage:
  database_name: harness   # default for auth-credentials and provider-config

auth_credentials:
  database_name: harness   # optional override (wins over storage.database_name)
```

- `storage.database_name` (default `harness`) -- shared logical pool for
  every harness `DbStore` consumer.
- `auth_credentials.database_name` (optional) -- per-worker override;
  must match a key in the engine `database` worker's `databases:` map.

## Dependencies

The `database` worker must be on the bus -- every read/write routes
through `database::query` / `database::execute`. See
[src/auth-credentials/iii.worker.yaml](harness/src/auth-credentials/iii.worker.yaml).

## Source layout

| File | Purpose |
|---|---|
| [src/auth-credentials/main.ts](harness/src/auth-credentials/main.ts) | Binary entry point (`iii-auth-credentials`). |
| [src/auth-credentials/register.ts](harness/src/auth-credentials/register.ts) | Composes the five handlers around a `DbCredentialStore`; runs the JSON-import migration. |
| [src/auth-credentials/config.ts](harness/src/auth-credentials/config.ts) | Loads the `auth_credentials` section. |
| [src/auth-credentials/types.ts](harness/src/auth-credentials/types.ts) | `Credential`, `AuthStatus`, `AuthSource`, `EnvKeyMatch`, and the `ENV_VAR_MAP` per-provider env table. |
| [src/auth-credentials/store.ts](harness/src/auth-credentials/store.ts) | `CredentialStore` interface + `InMemoryStore` + `DbCredentialStore` (wraps the shared `DbStore<Credential>`). |
| [src/runtime/database-store.ts](harness/src/runtime/database-store.ts) | Shared SQL-backed key-value store used by both `auth-credentials` and `provider-config`. |
| [src/auth-credentials/resolve.ts](harness/src/auth-credentials/resolve.ts) | Pure `resolveCredential` / `statusFor` / `findEnvKeys` helpers. |
| [src/auth-credentials/handlers/get-token.ts](harness/src/auth-credentials/handlers/get-token.ts) | `auth::get_token` handler. |
| [src/auth-credentials/handlers/set-token.ts](harness/src/auth-credentials/handlers/set-token.ts) | `auth::set_token` handler. |
| [src/auth-credentials/handlers/delete-token.ts](harness/src/auth-credentials/handlers/delete-token.ts) | `auth::delete_token` handler. |
| [src/auth-credentials/handlers/list-providers.ts](harness/src/auth-credentials/handlers/list-providers.ts) | `auth::list_providers` handler. |
| [src/auth-credentials/handlers/status.ts](harness/src/auth-credentials/handlers/status.ts) | `auth::status` handler. |
| [src/auth-credentials/iii.worker.yaml](harness/src/auth-credentials/iii.worker.yaml) | Worker manifest. |
