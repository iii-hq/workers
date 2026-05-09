# approval::list_pending

Return every unresolved approval envelope for one session prefix so dashboards can hydrate after reload.

`(payload) → { pending: [...] }` — pass `{ "session_id": "<sid>" }`. Empty `session_id` returns `pending: []`.

## When to use

- Hydrate the approvals rail when the SPA boots or reconnects after focus changes.

## Notes

Only rows whose `status` is still `"pending"` are returned; resolved rows stay in state until pruned externally.
