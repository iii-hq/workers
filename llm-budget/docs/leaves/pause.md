# Pausing or resuming a budget

## When to use

- Suspending enforcement during a maintenance window or planned high-spend batch operation.
- Resuming a budget after a pause window has ended.
- Differentiating between "monitoring only" (`enforced = false`) and "temporarily suspended" (`paused = true`) for operational clarity.

## Notes

- `paused` takes precedence over `enforced`: a paused budget with `enforced = true` still returns `allowed: true` from `budget::check`.
- Spend continues to accumulate via `budget::record` while paused, and alerts can still fire — only the ceiling check is bypassed.
- `budget::update` can also toggle `paused` alongside other field changes; this function is the single-purpose shorthand.
- Changes take effect immediately for subsequent `budget::check` calls.
