# budget::check

Check whether a budget allows an estimated spend before making an LLM call.

`({ budget_id, estimated_cost_usd?, principal_id? }) → { allowed: bool, remaining_usd: f64, reason? }` —
rolls forward the period if stale, prunes expired exemptions, then returns whether the estimated
cost fits within the remaining ceiling. `estimated_cost_usd` defaults to `0` (existence check).
`reason` is present only when `allowed` is `true` due to a special condition or `false`.

## When to use

- Gating an LLM call: if `allowed` is `false`, abort and surface an over-budget error to the user.
- Checking headroom before committing to a long multi-turn conversation.
- Verifying that a specific principal (`principal_id`) is not exempt before applying the cap.

## Notes

- `check` does **not** record spend — it is a read-with-side-effects operation. Always follow a
  successful check with `budget::record` once the actual cost is known.
- `reason` values when `allowed = true`: `"paused"` (budget is paused, cap not applied),
  `"not_enforced"` (enforcement disabled), `"exempt"` (principal matches an active exemption),
  or absent (normal headroom available).
- `reason` when `allowed = false`: `"ceiling_exceeded"` (estimated cost exceeds remaining headroom).
- The call mutates state only if period rollover or exemption pruning is needed; the budget record
  is saved only when it changes.
