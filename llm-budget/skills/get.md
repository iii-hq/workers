# budget::get

Fetch a single budget record by ID.

`({ budget_id }) → { budget: Budget }` — loads and returns the full budget object including
`spent_usd`, `ceiling_usd`, `period`, `period_start_at`, `period_resets_at`, `enforced`,
`paused`, `alerts`, and `exemptions`. Errors if the budget does not exist.

## When to use

- Inspecting the current state of a specific budget before making a routing or enforcement decision.
- Reading alert and exemption lists for a known budget ID.
- Verifying that a `budget::create` or `budget::update` call took effect as expected.

## Notes

- Does not roll forward the period or mutate any state — pure read.
- To get spend totals aggregated across multiple periods, use `budget::usage` instead.
- The `alerts` array contains threshold percentages and callback function IDs; `exemptions` contain
  principal IDs and expiry timestamps.
