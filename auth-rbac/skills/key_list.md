# auth::rbac::key_list

List all API keys in a workspace, ordered by creation time descending.

`({ workspace_id }) → { keys: [{ key_id, role, description, created_at, last_used_at, revoked_at }] }` —
returns metadata for every key in the workspace. Revoked keys are included;
filter on `revoked_at != null` to exclude them. Hashes are never returned.

## When to use

- Auditing which keys exist for a workspace.
- Showing an API key management UI to an admin or owner.
- Identifying keys that have not been used recently before revoking them.

## Notes

- `last_used_at` is updated lazily (at most once per 5 minutes per key) to avoid write amplification on every verify call.
- To revoke a specific key, pass its `key_id` to `auth::rbac::key_revoke`.
