# auth::rbac::role_list

List all role grants in a workspace, ordered by grant time ascending.

`({ workspace_id }) → { grants: [{ user_id, role, granted_at }] }` — returns every
role grant record for the workspace. Grants are sorted by `granted_at` ascending
(oldest first).

## When to use

- Displaying workspace members and their roles in a management UI.
- Auditing who has access to a workspace before revoking or changing grants.
- Enumerating team members to send notifications or perform bulk operations.

## Notes

- `granted_at` is a Unix timestamp in milliseconds.
- The workspace owner's grant (set by `workspace_create`) is included in the list.
- Role grants for user identities are distinct from API key roles; to list keys, use `auth::rbac::key_list`.
