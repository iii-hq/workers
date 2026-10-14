# Changelog

## 0.1.0

- First release: the **ultracode** orchestrator, the hidden **ultracode-worker** and **ultracode-verifier** leaves, and two routing tiers, **ultracode-impl** (code changes) and **ultracode-qa** (the In review wave).
- Kanban tickets are the work queue and the live monitor; each ticket's assignee picks the agent tier that works it.
- Fan-out with `harness::spawn` in bounded waves, fan-in through one `state::barrier` wake per wave with a deadline, adversarial verification, bounded rounds, a completeness critic and an optional soft budget.
- Requires `harness` `^1.8.53`, `state` `>=0.22.18` and the first `kanban` release with `kanban::ticket::links` and `add_labels` (`^0.1.21`). Does not use the `workflow` worker.
