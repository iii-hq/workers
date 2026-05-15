# approval::sweep_session

Sweep all pending approval records for a session to `timed_out` with reason `session_deleted`.

**Payload:**
- `session_id` (string, required)

**Returns:**
- `{ ok: true, swept: <count> }`
- `{ ok: false, error: "missing_session_id", swept: 0 }`

**Behavior:**
- Only records with `status: "pending"` are flipped.
- Non-pending records (already resolved, executed, denied, etc.) are left untouched.
- Intended to be called by the session worker or turn-orchestrator when a session is being deleted, so that pending approvals don't dangle forever.
