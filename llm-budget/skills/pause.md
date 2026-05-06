# budget::pause

Pause or resume a budget, suspending ceiling enforcement while paused.

`({ budget_id, paused: bool }) → { budget_id, paused: bool }` — sets the `paused` flag atomically
inside a budget lock and returns the updated value. When `paused = true`, `budget::check` returns
`allowed: true` with `reason: "paused"` regardless of spend level and enforcement state.

## When to use

- Suspending enforcement during a maintenance window or planned high-spend batch operation.
- Resuming a budget after a pause period has ended.
- Differentiating between "monitoring only" (`enforced = false`) and "temporarily suspended"
  (`paused = true`) states for operational clarity.

## Notes

- `paused` takes precedence over `enforced`: a paused budget with `enforced = true` still returns
  `allowed: true` from `budget::check`.
- Spend continues to accumulate via `budget::record` even while paused. Alerts can still fire.
- `budget::update` can also toggle `paused` alongside other field changes; `budget::pause` is the
  single-purpose shorthand.
- Changes take effect immediately for subsequent `budget::check` calls.
