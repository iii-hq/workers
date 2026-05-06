# budget::update

Update a whitelisted set of fields on an existing budget.

`({ budget_id, patch: { name?, ceiling_usd?, period?, enforced?, paused? } }) → { budget: Budget }` —
applies a partial update. Only the fields `name`, `ceiling_usd`, `period`, `enforced`, and `paused`
are accepted; any other keys in `patch` are silently dropped. The full updated budget is returned.

## When to use

- Raising or lowering the ceiling on an existing budget without recreating it.
- Switching a budget's rollover period (e.g., from `"day"` to `"month"`) after a policy change.
- Toggling `enforced` or `paused` flags alongside other field changes in a single atomic call.

## Notes

- **Period change**: if `period` changes, the current window is archived as a spend log entry and
  `spent_usd` is reset to `0.0` before the new period is anchored. Alert `last_fired_period_start`
  fields are also cleared.
- **Atomic**: the update runs inside a budget lock; concurrent `budget::record` calls will queue
  behind it.
- For toggling only `enforced` or only `paused`, the dedicated `budget::enforce` and `budget::pause`
  functions are more explicit but semantically equivalent.
- `ceiling_usd` must be positive and finite; `period` must be `"day"`, `"week"`, or `"month"`.
