# approval::list_undelivered

Return resolved approval records for a session that haven't been stamped with `delivered_in_turn_id`. Read-only — does not stamp anything. For at-most-once delivery semantics, prefer `approval::consume_undelivered`.

**Payload:**
- `session_id` (string, required)
- `limit` (number, optional, default 50) — maximum entries returned in a single call.

**Returns:**
- `{ entries: [Record, …], omitted: <count> }` — each entry has `status ∈ {executed, failed, denied, timed_out}`, plus optional `result`, `error`, `decision_reason`, and `resolved_at` (ms-since-epoch the record reached its terminal status).

**Ordering:**
- Entries are returned **oldest-first** by `resolved_at`. Records missing `resolved_at` sort last.
- When the unstamped set exceeds `limit`, the older entries are returned and `omitted` reports how many were left behind.

**Side effects (lazy migration / timeout):**
- Pending records past `expires_at` are flipped to `timed_out` and surfaced in the same call.
- Legacy records (old `status: "allow" | "deny"`) are migrated to `executed`/`denied` with a `legacy_migrated: true` marker.

Pair with `approval::ack_delivered` after a successful LLM turn to stamp these records and prevent re-delivery, or use `approval::consume_undelivered` to atomically list+stamp in one call.
