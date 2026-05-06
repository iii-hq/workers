# budget::alert_set

Add a spend-threshold alert to a budget that fires a callback when crossed.

`({ budget_id, threshold_pct, callback_function_id, callback_payload? }) → { alert_id }` —
appends a new `Alert` with a fresh UUID to the budget's alert list. `threshold_pct` is the
fraction of the ceiling (e.g., `0.8` for 80%) at which the alert fires. `callback_function_id`
is the iii function to invoke. `callback_payload` is an optional object merged under the system
fields in the callback invocation.

## When to use

- Setting up an 80% warning alert so an operator is notified before the ceiling is hit.
- Configuring a 100% alert to trigger automatic agent suspension when the budget is exhausted.
- Registering multiple thresholds (e.g., 50%, 80%, 100%) with different callback targets.

## Notes

- `threshold_pct` must be in the range `(0, 1]` (exclusive lower bound, inclusive upper bound).
- Each alert fires at most once per budget period. `last_fired_period_start` is set to the
  current period start when fired; it resets to `null` on `budget::reset` or period rollover.
- The callback receives: `{ alert_id, budget_id, spent_usd, ceiling_usd, threshold_pct, ...callback_payload }`.
  System fields always override any same-named keys in `callback_payload`.
- To remove an alert, use `budget::update` to patch the `alerts` array (or recreate the budget).
