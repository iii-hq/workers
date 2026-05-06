# budget::reset

Reset a budget's current period spend to zero and archive the prior window.

`({ budget_id }) → { budget_id, previous_spent_usd }` — rolls forward any stale zero-spend periods
first, archives the current window as a spend log entry (with a unique UUID suffix to avoid key
collisions), re-anchors `period_start_at` and `period_resets_at` to the current period, sets
`spent_usd = 0.0`, and clears `last_fired_period_start` on all alerts.

## When to use

- Manually resetting a budget mid-period after an anomalous spend event that should not count.
- Forcing a clean slate before a planned high-volume operation with prior approval.
- Correcting a budget that accumulated spend from a misconfigured agent run.

## Notes

- The budget record is saved **before** the archive log entry. If the archive write fails, the
  reset has already committed; the error is logged but not rethrown, so the caller should not
  retry unconditionally.
- `previous_spent_usd` in the response reflects the spend that was archived — useful for audit
  or notification purposes.
- Period rollover (automatic, time-based) is separate: `reset` is an explicit manual operation
  that does not affect the automatic `period_resets_at` boundary.
