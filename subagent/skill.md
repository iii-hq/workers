# subagent

Spawn focused child agent runs on the iii bus via `run::start_and_wait`, then
return the final assistant text to the parent tool caller. Useful for isolating
subtasks, alternative reasoning paths, or depth-bounded nested agents.

- [`subagent`](iii://subagent)
  - [`subagent::start`](iii://subagent/start) — spawn a child session and block until terminal
