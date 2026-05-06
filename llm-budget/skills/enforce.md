# budget::enforce

Toggle enforcement on a budget without changing any other fields.

`({ budget_id, enforced: bool }) → { budget_id, enforced: bool }` — sets the `enforced` flag
atomically inside a budget lock and returns the updated value. When `enforced = false`,
`budget::check` returns `allowed: true` with `reason: "not_enforced"` regardless of spend level.

## When to use

- Temporarily disabling enforcement to allow a burst of spend for a planned operation,
  then re-enabling it afterward.
- Disabling enforcement on a monitoring-only budget where you want to track spend without blocking.
- Restoring enforcement after the reason for disabling it has passed.

## Notes

- Enforcement state is independent of `paused`: a budget can be `enforced = false, paused = false`
  (tracking only), `enforced = true, paused = true` (paused but would enforce if resumed), or any
  other combination.
- `budget::update` can also set `enforced` as part of a multi-field patch; `budget::enforce` is
  the single-purpose shorthand.
- Changes take effect immediately for subsequent `budget::check` calls.
