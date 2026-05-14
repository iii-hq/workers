# approval-gate

Subscriber on `agent::before_function_call` that holds calls whose ids appear in `approval_required`, streams `approval_requested` events, and clears when the UI invokes `approval::resolve` or the timeout lapses.

- [`approval-gate`](iii://approval-gate/index)
  - [`approval::resolve`](iii://approval-gate/resolve) — record `allow` or `deny` for a blocked function call ID in a session
  - [`approval::list_pending`](iii://approval-gate/list_pending) — refreshable list of unresolved pending calls per session for the UI
  - [`policy::approval_gate`](iii://approval-gate/policy_approval_gate) — hook handler bound to `durable:subscriber` on your configured topic; do not invoke directly unless probing registration
