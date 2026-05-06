# budget::delete

Permanently delete a budget record and its state.

`({ budget_id }) → { ok: true }` — removes the budget from the state store. The operation runs
inside a budget lock to prevent concurrent modifications. Idempotent in practice: if the budget
is already absent, the underlying state delete is a no-op.

## When to use

- Decommissioning a budget that is no longer needed (e.g., an agent was retired).
- Cleaning up test or trial budgets after a demonstration.
- Replacing a budget by deleting the old one and creating a new one with different parameters.

## Notes

- Spend log entries (archived periods) are **not** deleted. Historical spend data persists
  under the old `budget_id` until the state store is independently purged.
- Alerts and exemptions attached to the budget are implicitly removed with the budget record.
- There is no soft-delete or recovery path: once deleted, the `budget_id` is gone.
