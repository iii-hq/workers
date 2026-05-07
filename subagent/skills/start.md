# subagent::start

Spawn a child durable agent session and wait until it reaches a terminal state,
then return the last non-empty assistant text from the child transcript.

`({ prompt, provider, model, system_prompt?, parent_session_id?, max_subagent_depth? }) → { content, details }` — mints `child_session_id` as `{parent_session_id}::sub-{timestamp_ms}` (parent defaults to `root`), calls `run::start_and_wait` with that session, a single user message built from `prompt`, and the given `provider` / `model` / `system_prompt`. On success: `content` is a tool-shaped array with one `text` block; `details` includes `child_session_id`, `turn_count`, and `via: "run::start_and_wait"`. On depth refusal: `details.depth_limit_reached` is true and `content` explains the cap.

## When to use

- The parent agent wants a concise answer to a narrow sub-question without bloating its own scratchpad.
- You need a one-shot nested run with a different default system prompt than the parent.
- You are experimenting with an alternative model or provider for the same prompt shape.
- You must respect a nesting cap: pass a chain in `parent_session_id` that already contains `::sub-` segments so depth is counted correctly.

## Notes

- The child run does **not** take its tool list from this payload. `turn-orchestrator` rebuilds the LLM tool catalog from `engine::functions::list` during the provisioning state, so the child sees the **global** catalog (including `subagent::start`). Use `max_subagent_depth` (worker default 3, overridable per call) to cap recursion: depth is `parent_session_id.matches("::sub-").count()` compared to `max_subagent_depth`.
- Pass the parent harness session id in `parent_session_id` when nesting so depth is tracked across spawns; omit or use `root` only for a top-level sub-agent from an unknown parent chain.
- `trigger_timeout_ms` in the worker config (default 600000) bounds how long `run::start_and_wait` may block before the trigger fails.
- To cap turns on the child run, extend the `run::start` payload contract in `turn-orchestrator` (e.g. `max_turns` on start); `subagent::start` does not read `max_turns` today.
