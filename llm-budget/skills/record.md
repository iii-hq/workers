# budget::record

Record an actual LLM spend and fire any threshold alerts that have been crossed.

`({ budget_id, cost_usd }) → { spent_usd: f64, remaining_usd: f64 }` — rolls forward the period
if stale, adds `cost_usd` to `spent_usd`, checks all configured alerts, and fires those whose
`threshold_pct` is newly crossed in the current period (each alert fires at most once per period).
Alert callbacks are dispatched via `iii.trigger` in background tasks (fire-and-forget).

## When to use

- Immediately after an LLM call returns, to record the actual token cost.
- Recording a batch cost after a sequence of sub-calls whose total is known.
- Updating spend from an external billing event that maps to a known budget.

## Notes

- `cost_usd` must be a finite number ≥ 0. Non-finite or negative values are rejected.
- The operation runs inside a budget lock; concurrent `budget::record` calls for the same budget
  are serialized. Do not call in a tight loop under high concurrency — batch costs before recording.
- Alert callback payloads include `alert_id`, `budget_id`, `spent_usd`, `ceiling_usd`, and
  `threshold_pct` merged on top of the alert's optional `callback_payload` object.
- This function does not enforce the ceiling — it records unconditionally. Call `budget::check`
  before the LLM call to enforce the cap proactively.
