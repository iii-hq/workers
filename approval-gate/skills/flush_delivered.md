# approval::flush_delivered

One-shot drain: stamp every unacked terminal-status record in a session as delivered. Intended for operator recovery when a backlog has accumulated (e.g. after the orchestrator was offline, or while the pre-`consume_undelivered` redelivery bug was active). Does NOT touch pending records — use `approval::sweep_session` for that first if you need to expire still-pending approvals before draining.

**Payload:**
- `session_id` (string, required)
- `turn_id` (string, required) — sentinel value to stamp into `delivered_in_turn_id`. Conventionally `manual-flush-<ts>` so audits can see the records were drained out-of-band rather than delivered to a real LLM turn.

**Returns:**
- `{ ok: true, stamped: <count> }`
- On missing payload fields: `{ ok: false, error: "missing_session_or_turn_id", stamped: 0 }`.

**Behavior:**
- Iterates the session's records via prefix scan.
- Skips records that are pending (still awaiting a human decision) or already stamped with a non-null `delivered_in_turn_id`.
- For each remaining terminal record (`executed | failed | denied | timed_out`), writes `delivered_in_turn_id = turn_id`.

**When to call:**
- Before starting an agent on a session that has stale backlog older than the agent should see.
- After confirming via `approval::list_undelivered` that the unacked count is too large to surface organically (e.g. hundreds of entries).

Idempotent: subsequent calls re-scan but stamp nothing (already-stamped records are skipped).
