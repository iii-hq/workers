# approval::list_pending

Return currently-pending approval records for a session, used by UI hydration on browser reconnect.

**Payload:**
- `session_id` (string, required)

**Returns:**
- `{ pending: [Record, …] }` — only `status: "pending"` records; legacy records and expired pendings are filtered.

For agent-facing turn integration use `approval::list_undelivered` instead.
