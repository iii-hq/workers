# auth-rbac

HMAC API keys and workspace roles (owner/admin/member/viewer) on the iii bus.
Distinct from `auth-credentials` (provider token vault) — these crates share no
state and their function-id namespaces (`auth::*` vs `auth::rbac::*`) are
disjoint by construction.

## Installation

```bash
iii worker add auth-rbac
```

## Run

```bash
AUTH_HMAC_SECRET="<base secret>" iii-auth-rbac --engine-url ws://127.0.0.1:49134
```

`AUTH_HMAC_SECRET` is required — keys are HMACed with it on creation and
verification. Rotate by re-keying every active key (no automatic key
migration today).

## Registered functions

| Function | Description |
|---|---|
| `auth::rbac::workspace_create` | Create a workspace; grants the owner role to the creator. |
| `auth::rbac::workspace_get` | Read a workspace by id. |
| `auth::rbac::key_create` | Mint an API key (returns the plaintext token once). |
| `auth::rbac::key_list` | List keys (hashed) for a workspace. |
| `auth::rbac::key_revoke` | Revoke a key by id. |
| `auth::rbac::verify` | Verify a presented token; returns the workspace and role. |
| `auth::rbac::role_grant` | Grant a role to a subject. |
| `auth::rbac::role_check` | Check whether a subject has at least the required role. |
| `auth::rbac::role_list` | List role grants for a workspace. |

## Build

```bash
cargo build --release
```

The binary is `iii-auth-rbac`.
