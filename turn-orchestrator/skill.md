# turn-orchestrator

Durable agent-turn state machine on the iii bus. Drives each session
through provisioning → assistant → tools → steering → tearing-down,
checkpointing state on every step so an interrupted run resumes from
the last persisted node.

- [`turn-orchestrator`](iii://turn-orchestrator)
  - [`run::start`](iii://turn-orchestrator/run-start) — start a durable run, return session id immediately
  - [`run::start_and_wait`](iii://turn-orchestrator/run-start-and-wait) — start and block until terminal (test/dev)
