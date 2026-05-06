# auth-credentials

Provider credential vault on the iii bus. Stores API keys and OAuth tokens
under `auth::*`.

## Installation

```bash
iii worker add auth-credentials
```

## Run

```bash
iii-auth-credentials --engine-url ws://127.0.0.1:49134
```

Defaults to an iii-state-backed store (credentials survive worker restart).
Set `AUTH_CREDENTIALS_STORE=memory` for ephemeral in-process storage; see
[Storage backends](#storage-backends) below.

## Registered functions

| Function | Description |
|---|---|
| `auth::set` | Store a credential for a provider. |
| `auth::get` | Read a credential for a provider. |
| `auth::list` | List provider credentials. |
| `auth::clear` | Remove a credential. |
| `auth::resolve` | Resolve effective credential (refresh if needed). |

## Storage backends

`auth-credentials` supports two storage backends, selected via the
`AUTH_CREDENTIALS_STORE` env var:

| Value | Persistence | Use case |
|---|---|---|
| `iii_state` (default) | Survives worker restart via `iii-state` | Production |
| `memory` | In-process only; lost on restart | Tests, local dev |

The `iii_state` backend stores each credential under scope `auth_credentials`,
key `credential:<provider>`, value `{ "provider": "<provider>", "credential": <Credential> }`.
The provider name is embedded in the value so `auth::list_providers` can
recover the `(provider, credential)` tuple list (`state::list` returns values
without keys).

### Failure semantics

`CredentialStore::{get, set, clear, list}` return `anyhow::Result<...>`. With
the `iii_state` backend, transient bus failures (engine restart, IPC hiccup)
surface as `Err` to the bus caller. With `memory`, only `RwLock` poison
errors are surfaced — under normal use the methods are infallible.

Callers of `auth::get_token` should expect a bus error response on transient
state failures; retry policy is the caller's choice.

## Build

```bash
cargo build --release
```
