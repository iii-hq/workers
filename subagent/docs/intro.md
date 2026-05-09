Spawn a focused child agent run on the iii bus via `run::start_and_wait`, block until it reaches a terminal state, then return the last non-empty assistant text to the parent tool caller. Use it to isolate a sub-question into its own session, try an alternative model on the same prompt shape, or run a depth-bounded nested agent without bloating the parent's scratchpad.

<!-- llm-only:start -->
The child run rebuilds its tool catalog from `engine::functions::list` during provisioning — it does not inherit the parent's tool list and sees the global catalog including `subagent::start` itself. Pass `max_subagent_depth` (worker default 3) to cap recursion. Depth is counted as `parent_session_id.matches("::sub-").count()`, so always thread the real `parent_session_id` through when nesting.
<!-- llm-only:end -->
