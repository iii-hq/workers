# Spawning a child agent and waiting for its answer

## When to use

- The parent agent needs a concise answer to a narrow sub-question without polluting its own scratchpad.
- A one-shot nested run that should use a different default system prompt from the parent.
- Comparing an alternative model or provider on the same prompt shape.
- Bounded nesting: pass a `parent_session_id` whose chain already contains `::sub-` segments so depth is counted correctly.

## Notes

- The child does not take its tool list from the payload. `turn-orchestrator` rebuilds the catalog from `engine::functions::list` during provisioning, so the child sees the **global** catalog including `subagent::start`. Cap recursion with `max_subagent_depth` (worker default 3); depth is `parent_session_id.matches("::sub-").count()`.
- Always pass the real `parent_session_id` when nesting so depth tracks across spawns. Use `root` (or omit) only for a top-level sub-agent with no parent chain.
- `trigger_timeout_ms` in `config.yaml` (default 600000) bounds how long `run::start_and_wait` may block before the trigger fails.
- The child session id is minted as `{parent_session_id}::sub-{timestamp_ms}` and returned in `details.child_session_id`. On a depth refusal, `details.depth_limit_reached` is true and `content` explains the cap.
- To cap turns on the child run, extend the `run::start` contract in `turn-orchestrator` — `subagent::start` does not read `max_turns` today.
