# budget::usage

Aggregate historical spend over a time window, optionally across archived periods.

`({ budget_id, window? }) → { spent_usd: f64, by_period: [{period, spent}], records_count: u64 }` —
rolls forward the period if stale, loads archived spend logs, and sums spend within the requested
window. `window` defaults to `"all"`; valid values are `"all"`, `"day"`, `"week"`, or `"month"`.
The window must match the budget's configured period, or be `"all"`.

## When to use

- Generating a spend report for a workspace or agent over the current or historical periods.
- Checking cumulative spend across all time before deciding whether to renew or cap a budget.
- Comparing period-by-period spend to identify usage spikes.

## Notes

- `by_period` is sorted ascending by `period` (period start timestamp in milliseconds).
  The live (current) period is always appended last.
- A window value that does not match the budget's period returns an error with a suggestion
  (e.g., `"window 'day' does not align with budget period 'month'. Use window: 'month' or 'all'"`).
- The live period's spend is taken from the budget record; archived periods are taken from
  spend log entries. The current period is excluded from the log query to prevent double-counting
  after a reset or rollover.
- For cost-per-token data needed to estimate future spend, see [`models-catalog`](iii://models-catalog).
