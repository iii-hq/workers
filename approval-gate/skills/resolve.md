# approval::resolve

Apply a workspace operator decision to unblock (or permanently block) one pending tool call keyed by `(session_id, function_call_id)`.

`(input) → { ok, error? }` — send `session_id`, `function_call_id` (legacy `tool_call_id` alias), string `decision` (`allow` | `deny`), and optional `reason` text for denies. Entries must currently be `status: pending` in state.

## When to use

- The chat shell shows an approval row tied to `approval_requested`; the UI calls this after the user confirms or rejects the action.

## Notes

Requires the `state::*` primitives on the bus and the configured `approval_state_scope` matching this worker (`config.yaml` / operator manifest).
