# auth-credentials

Multi-provider credential store on the iii bus (`auth::*`).

## Purpose

Centralised storage for provider credentials — API keys and OAuth tokens
— used by the provider workers (`provider-anthropic`, `provider-openai`)
to authenticate outbound calls. Stored credentials live in a single JSON
file at `~/.iii/auth-credentials.json` (path configurable). All mutations
serialize through an in-process write queue and use atomic `tmp` +
`rename` writes.

`auth::get_token` is the resolver every consumer calls. It first looks up
a stored credential; if none, it falls back to the per-provider
environment variable defined in
[src/auth-credentials/types.ts](harness/src/auth-credentials/types.ts)
(`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `AWS_ACCESS_KEY_ID`,
`HF_TOKEN`, etc.) and synthesises an `ApiKey` credential. `auth::status`
reports `{ configured, source: "stored" | "environment", label }` without
returning the secret material itself.

[iii-permissions.yaml](iii-permissions.yaml) denies
`auth::set_token` and `auth::delete_token` to in-run agents (kernel
rules). `auth::get_token`, `auth::list_providers`, and `auth::status` are
on the bare-string allow list.

## Registered functions

- `auth::get_token` — Fetch the stored credential for a provider.
- `auth::set_token` — Persist a credential for a provider.
- `auth::delete_token` — Remove the stored credential for a provider.
- `auth::list_providers` — List every provider with a stored credential.
- `auth::status` — Report stored vs. env credential status for a provider.

## Triggers

None.

## On-disk layout

| Path | Format |
|---|---|
| `~/.iii/auth-credentials.json` (configurable) | `{ "<provider>": <Credential>, … }` with `<Credential>` being either `{ type: "api_key", key }` or `{ type: "oauth", access_token, refresh_token?, expires_at?, scopes?, provider_extra? }`. File mode 0600, parent dir 0700. |

## Configuration

From the `auth_credentials` section of
[config.yaml](harness/config.yaml):

- `store_path` (default `~/.iii/auth-credentials.json`) — leading `~/` is
  expanded against the OS home directory.

## Dependencies

From
[src/auth-credentials/iii.worker.yaml](harness/src/auth-credentials/iii.worker.yaml):
no explicit dependency block; the worker only touches the local
filesystem and the process environment.

## Source layout

| File | Purpose |
|---|---|
| [src/auth-credentials/main.ts](harness/src/auth-credentials/main.ts) | Binary entry point (`iii-auth-credentials`). |
| [src/auth-credentials/register.ts](harness/src/auth-credentials/register.ts) | Composes the five handlers around a single `FileStore`. |
| [src/auth-credentials/config.ts](harness/src/auth-credentials/config.ts) | Loads the `auth_credentials` section. |
| [src/auth-credentials/types.ts](harness/src/auth-credentials/types.ts) | `Credential`, `AuthStatus`, `AuthSource`, `EnvKeyMatch`, and the `ENV_VAR_MAP` per-provider env table. |
| [src/auth-credentials/store.ts](harness/src/auth-credentials/store.ts) | `CredentialStore` interface + `InMemoryStore` + `FileStore` (atomic tmp+rename, in-process queue). |
| [src/auth-credentials/resolve.ts](harness/src/auth-credentials/resolve.ts) | Pure `resolveCredential` / `statusFor` / `findEnvKeys` helpers. |
| [src/auth-credentials/handlers/get-token.ts](harness/src/auth-credentials/handlers/get-token.ts) | `auth::get_token` handler. |
| [src/auth-credentials/handlers/set-token.ts](harness/src/auth-credentials/handlers/set-token.ts) | `auth::set_token` handler. |
| [src/auth-credentials/handlers/delete-token.ts](harness/src/auth-credentials/handlers/delete-token.ts) | `auth::delete_token` handler. |
| [src/auth-credentials/handlers/list-providers.ts](harness/src/auth-credentials/handlers/list-providers.ts) | `auth::list_providers` handler. |
| [src/auth-credentials/handlers/status.ts](harness/src/auth-credentials/handlers/status.ts) | `auth::status` handler. |
| [src/auth-credentials/iii.worker.yaml](harness/src/auth-credentials/iii.worker.yaml) | Worker manifest. |
