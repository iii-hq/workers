# budget::forecast

Project spend through the end of the current period based on the current daily burn rate.

`({ budget_id }) → { projected_month_usd: f64, on_track: bool, days_until_breach: f64? }` —
rolls forward the period if stale, computes the daily spend rate (`spent_usd / days_elapsed`),
then projects that rate over 30 days (`projected_month_usd`) and through the remaining period
(`on_track = rate * remaining_days <= remaining_budget`). `days_until_breach` is set only when
rate > 0 and remaining headroom > 0.

## When to use

- Determining whether a budget is on track to stay under the ceiling before the period resets.
- Alerting an operator early (e.g., at 50% elapsed time) that the projected total will overshoot.
- Building a spend forecast widget that shows estimated end-of-period cost.

## Notes

- The rate is computed from the current period's `spent_usd` and `days_elapsed`; it does not
  incorporate archived periods. If the budget was just created or reset, `days_elapsed` may be
  very small, producing a high projected rate from a small sample — treat early forecasts with
  appropriate uncertainty.
- `projected_month_usd` is always 30-day projection regardless of the budget's `period` setting.
- `days_until_breach` is `null` (absent) when the rate is zero (no spend yet) or when there is
  no remaining headroom (already over ceiling).
- For actual historical spend breakdown, use `budget::usage`.
