# auth-rbac

Workspace access-control layer: mint and verify HMAC API keys, manage workspaces, and enforce role-based permissions (`owner / admin / member / viewer`).

- [`auth-rbac`](iii://auth-rbac)
  - [`auth::rbac::workspace_create`](iii://auth-rbac/workspace_create) — create a workspace and grant owner role
  - [`auth::rbac::workspace_get`](iii://auth-rbac/workspace_get) — fetch workspace metadata by ID

  - [`auth::rbac::key_create`](iii://auth-rbac/key_create) — mint an HMAC API key bound to a workspace + role
  - [`auth::rbac::key_list`](iii://auth-rbac/key_list) — list all keys in a workspace
  - [`auth::rbac::key_revoke`](iii://auth-rbac/key_revoke) — revoke a key by ID (idempotent)
  - [`auth::rbac::verify`](iii://auth-rbac/verify) — validate a token, optionally asserting workspace + role

  - [`auth::rbac::role_grant`](iii://auth-rbac/role_grant) — grant or update a user's role in a workspace
  - [`auth::rbac::role_check`](iii://auth-rbac/role_check) — check whether a user meets a minimum role requirement
  - [`auth::rbac::role_list`](iii://auth-rbac/role_list) — list all role grants in a workspace

For provider credential storage (API keys and OAuth tokens), see [`auth-credentials`](iii://auth-credentials).
