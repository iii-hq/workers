Session storage on the iii bus as a parent-id DAG of typed entries — agent messages, branch summaries, custom payloads, and compaction markers. `session-tree::*` lets callers create sessions, append turn-by-turn, fork branches at any entry, clone whole histories, and export a self-contained HTML transcript of the active path. Forks share history because every entry references its parent rather than copying it forward.

<!-- llm-only:start -->
For resuming an agent, prefer `session-tree::messages` over `session-tree::tree` — `messages` returns only the active path in oldest-first order, which slots straight into the agent's context. `tree` returns the entire DAG including sibling branches and is much heavier on large sessions.
<!-- llm-only:end -->
