# budget::list

List all budgets, optionally filtered by workspace, newest first.

`({ workspace_id? }) → { budgets: Budget[] }` — returns all stored budget records sorted by
`created_at` descending. If `workspace_id` is provided, only budgets with a matching
`workspace_id` are returned. Returns an empty array if none match.

## When to use

- Discovering which budgets exist before deciding which one to attach to a new agent or workflow.
- Auditing all budgets in a workspace to check current spend across cost centers.
- Building a dashboard view that needs to enumerate active vs. paused budgets.

## Notes

- Each returned `Budget` object includes `spent_usd`, `ceiling_usd`, `enforced`, `paused`, and period
  boundary timestamps — enough to assess status without a separate `budget::get` call.
- No pagination: the full set is returned in one response. For large deployments with many budgets,
  filter by `workspace_id` to reduce payload size.
