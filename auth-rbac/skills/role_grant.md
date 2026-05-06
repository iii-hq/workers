# auth::rbac::role_grant

Grant a workspace role to a user, or update an existing role grant.

`({ workspace_id, user_id, role }) → { ok: true }` — upserts the role grant record.
`role` must be one of `owner`, `admin`, `member`, or `viewer`. Refuses to demote the
workspace owner: if the target `user_id` is the workspace's `owner_id`, the role must
remain `owner`.

## When to use

- Adding a collaborator to a workspace with a specific role.
- Promoting a `member` to `admin` after a team change.
- Granting a service account `viewer` access for read-only operations.

## Notes

- The workspace must already exist; returns an error if `workspace_id` is not found.
- Cannot demote the workspace owner via this function — use an explicit ownership transfer flow.
- Role grants affect `role_check` immediately; API key roles are independent and set at mint time.
- Use `role_list` to audit all current grants in a workspace.
