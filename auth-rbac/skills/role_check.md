# auth::rbac::role_check

Check whether a user holds at least the required role in a workspace.

`({ workspace_id, user_id, required_role }) → { allowed: bool }` — looks up the user's
role grant and returns `{ allowed: true }` if their granted role satisfies the required
level in the `owner > admin > member > viewer` hierarchy. Returns `{ allowed: false }` if
no grant exists or the grant's role is below the required level.

## When to use

- Gate-checking a user's permission before allowing a write or admin operation.
- Middleware that maps user identity to workspace access level.
- Audit logic that needs to confirm a user's minimum role without exposing the full grant record.

## Notes

- `required_role` must be one of `owner`, `admin`, `member`, or `viewer`.
- Returns `{ allowed: false }` (not an error) when the user has no grant in the workspace.
- Role grants are set via `auth::rbac::role_grant`; API key roles are separate and checked via `auth::rbac::verify`.
