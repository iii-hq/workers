# auth-credentials

Provider credential vault on the iii bus. Stores API keys and OAuth tokens
under `auth::*`. Distinct from `auth-rbac` (workspace roles + HMAC keys).

## Installation

```bash
iii worker add auth-credentials
```

## Run

```bash
iii-auth-credentials --engine-url ws://127.0.0.1:49134
```

Defaults to an in-memory backend — credentials are lost on restart. Production
deployments swap in an iii-state-backed `CredentialStore`.

## Registered functions

| Function | Description |
|---|---|
| `auth::set` | Store a credential for a provider. |
| `auth::get` | Read a credential for a provider. |
| `auth::list` | List provider credentials. |
| `auth::clear` | Remove a credential. |
| `auth::resolve` | Resolve effective credential (refresh if needed). |

## Build

```bash
cargo build --release
```
