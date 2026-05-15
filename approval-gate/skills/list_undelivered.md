# approval::list_undelivered

Return resolved approval records for a session that haven't been stamped with `delivered_in_turn_id`. Driven by turn-orchestrator before each LLM turn.

**Payload:**
- `session_id` (string, required)

**Returns:**
- `{ entries: [Record, …] }` — each entry has `status ∈ {executed, failed, denied, timed_out}`, plus optional `result`, `error`, `decision_reason`.

**Side effects (lazy migration / timeout):**
- Pending records past `expires_at` are flipped to `timed_out` and surfaced in the same call.
- Legacy records (old `status: "allow" | "deny"`) are migrated to `executed`/`denied` with a `legacy_migrated: true` marker.

Pair with `approval::ack_delivered` after a successful LLM turn to stamp these records and prevent re-delivery.
