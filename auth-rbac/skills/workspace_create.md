# auth::rbac::workspace_create

Create a new workspace and automatically grant the `owner` role to its creator.

`({ name, owner_id }) → { workspace_id }` — generates a UUID for the workspace,
persists the workspace record, and writes the owner role grant atomically.
If the role grant write fails, the workspace record is rolled back.

## When to use

- Onboarding a new tenant or team into the system.
- An orchestration flow is provisioning an isolated workspace for a user.
- A signup flow has completed and needs a workspace to associate API keys and roles to.

## Notes

- `owner_id` receives the `owner` role automatically; no separate `role_grant` call is needed.
- The workspace `id` in the response is the UUID to pass to subsequent `key_create`, `role_grant`, and `role_list` calls.
- The workspace owner cannot be demoted via `role_grant`; use an explicit ownership transfer flow if needed.
