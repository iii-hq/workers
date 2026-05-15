# approval::consume_undelivered

Atomic list+ack. Returns the same FIFO-capped slice of resolved-but-undelivered records as `approval::list_undelivered`, AND stamps each one with `delivered_in_turn_id` before returning. Use this instead of the list→ack pair when you want at-most-once redelivery semantics for the LLM.

**Payload:**
- `session_id` (string, required)
- `turn_id` (string, required) — value to stamp into `delivered_in_turn_id`.
- `limit` (number, optional, default 50)

**Returns:**
- `{ ok: true, entries: [Record, …], omitted: <count> }`
- On missing `turn_id`: `{ ok: false, error: "missing_turn_id", entries: [], omitted: 0 }`.

**Why prefer this over list_undelivered + ack_delivered:**

The previous two-call pattern (`list` → LLM → `ack`) created an unbounded-redelivery bug: if the LLM call failed between the two RPCs, the entries were never acked and resurfaced on every subsequent turn, accumulating into the agent's context. Consume stamps before returning, so a caller crash after the response is at most an information-loss event (the entries are still terminal records inside the gate — their side-effects already executed).

Render `omitted` to the model via the turn-orchestrator's `omission_summary_message` helper so it knows older entries exist but are not in this turn's context. Drain them in one shot with `approval::flush_delivered` if appropriate.
