# approval-gate

Subscriber on `agent::before_function_call`. Pauses function calls whose id appears
in the run's `approval_required` list, emits `ApprovalRequested` onto
`agent::events/<session_id>`, and waits for the UI to call `approval::resolve`
(or for the configured timeout, default 5 minutes).

## Functions
- `approval::resolve { function_call_id, tool_call_id?, decision, reason? }` — flip a pending entry to `allow` or `deny` (`tool_call_id` accepted for backward compatibility).
- `approval::list_pending { session_id }` — return currently-blocked calls (used by the UI on tab refresh).

## Config (env)
- `APPROVAL_GATE_TIMEOUT_MS` — auto-deny timeout in ms (default `300000`).
