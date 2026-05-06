# auth::rbac::workspace_get

Fetch public-facing metadata for a workspace by its ID.

`({ workspace_id }) → { workspace: { id, name, created_at } | null }` — returns
the workspace object if found, or `null` if no workspace exists for that ID.
The `owner_id` field is intentionally omitted from the response.

## When to use

- Displaying workspace details in a UI or API response.
- Verifying that a workspace exists before creating keys or granting roles.
- Resolving a workspace ID to a human-readable name for logging or audit trails.
