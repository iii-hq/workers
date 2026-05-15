# approval::sweep_session

Sweep all pending approval records for a session to `timed_out`.

**Payload:**
- `session_id` (string, required)

**Returns:**
- `{ ok: true, swept: <count> }`
- `{ ok: false, error: "missing_session_id", swept: 0 }`

**Behavior:**
- Only records with `status: "pending"` are flipped.
- Non-pending records (already resolved, executed, denied, etc.) are left untouched.
- The flipped records carry no `Denial` — `status: "timed_out"` is self-describing per the Denial refactor. Callers that need to distinguish session-delete from run-stop sweeps should log that context in their own worker.
- Intended to be called by the session worker or turn-orchestrator when a session is being deleted or a run is stopped, so pending approvals don't dangle forever.
