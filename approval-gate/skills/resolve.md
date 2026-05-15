# approval::resolve

Resolve a pending approval entry.

**Payload:**
- `session_id` (string, required)
- `function_call_id` (string, required) — accepts legacy `tool_call_id`
- `decision` (string, required) — `"allow"` or `"deny"`
- `reason` (string, optional) — surfaced as `decision_reason` on deny

**Returns:**
- `{ ok: true }` on success
- `{ ok: false, error: "not_found" | "already_resolved" | "bad_decision" | "missing_id" | "timed_out" | "state_write_failed" }`

**Behavior:**
- On `allow`: gate invokes the underlying function via `iii.trigger` and records the outcome (`executed { result }` or `failed { error }`).
- On `deny`: records `denied { decision_reason }` and never invokes the function.
- On expired-pending: flips to `timed_out` and returns `{ ok: false, error: "timed_out" }`; late decision is not honored.

Resolution is async with respect to the agent's turn — the agent sees a `pending_approval` tool result immediately when the call is intercepted, and the outcome stitches into the agent's next turn via `approval::list_undelivered`.
