# approval::ack_delivered

Stamp resolved approval records with the LLM turn id that surfaced them.

**Payload:**
- `session_id` (string, required)
- `call_ids` (string[], required)
- `turn_id` (string, required)

**Returns:**
- `{ ok: true, stamped: <count> }`

**Behavior:**
- Idempotent: records already stamped are not overwritten; the first turn_id wins.
- Unknown call ids are silently skipped.
