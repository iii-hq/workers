# auth::rbac::key_create

Mint an HMAC-hashed API key bound to a workspace and role, returning the plaintext token once.

`({ workspace_id, role, description?, created_by? }) → { key_id, token }` — generates
a random token, stores the HMAC-SHA256 hash (never the plaintext), and writes a
lookup index. The plaintext `token` is returned exactly once in this response;
it cannot be recovered later. `role` must be one of `owner`, `admin`, `member`, or `viewer`.

## When to use

- Issuing an API key to a user or service account scoped to a specific workspace and role.
- Rotating an API key (create a new key, then revoke the old one via `key_revoke`).
- Provisioning a CI/CD token with `member` or `viewer` role for read-only access.

## Notes

- The workspace must already exist; returns an error if `workspace_id` is not found.
- Store the returned `token` immediately — the plaintext is never persisted and cannot be retrieved again.
- `description` and `created_by` are optional metadata fields for audit purposes.
- Verify tokens with `auth::rbac::verify`; list all keys for a workspace with `auth::rbac::key_list`.
