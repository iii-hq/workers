Durable agent-turn state machine on the iii bus. `run::start` drives each session through provisioning, assistant, functions, steering, and tearing-down, persisting the session record on every transition so an interrupted run resumes from the last checkpointed node rather than restarting from scratch. Every transition also emits an `AgentEvent` frame on the `agent::events` stream for live observers.

<!-- llm-only:start -->
Reach for `run::start` for production runs — it returns the session id immediately and the orchestrator drives the session asynchronously. `run::start_and_wait` blocks until the session reaches a terminal state and is meant for tests, sub-agents, and one-shot scripted invocations. Re-issuing `run::start` with the same `session_id` resumes the persisted run rather than starting a fresh one.
<!-- llm-only:end -->
