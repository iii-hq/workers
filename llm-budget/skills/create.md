# budget::create

Create a new budget with a spend ceiling and rollover period.

`({ ceiling_usd, period, workspace_id?, agent_id?, name? }) → { budget_id }` — allocates a new
budget record with `spent_usd = 0`, anchors `period_start_at` to the current period boundary,
and sets `enforced = true`, `paused = false` by default. `period` must be `"day"`, `"week"`, or
`"month"`. `ceiling_usd` must be a positive finite number.

## When to use

- Establishing a spend cap for a workspace or individual agent before routing traffic through it.
- Allocating separate daily or monthly budgets to different teams or cost centers.
- Setting up a trial budget with a low ceiling to gate access until billing is confirmed.

## Notes

- The returned `budget_id` (a UUID) is the stable handle for all subsequent calls.
- Enforcement begins immediately: call `budget::check` before LLM calls to respect the ceiling.
- To disable enforcement temporarily without deleting the record, use `budget::enforce` or `budget::pause`.
- `workspace_id` and `agent_id` are optional free-form labels; `budget::list` can filter by `workspace_id`.
