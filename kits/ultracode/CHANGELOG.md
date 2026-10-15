# Changelog

## 0.1.0

- First release: the **ultracode** orchestrator, the hidden **ultracode-worker** and **ultracode-verifier** leaves, and two routing tiers, **ultracode-impl** (code changes) and **ultracode-qa** (the In review wave).
- Kanban tickets are the work queue and the live monitor; each ticket's assignee picks the agent tier that works it.
- Fan-out with `harness::spawn` in bounded waves, fan-in through one `state::barrier` wake per wave with a deadline, adversarial verification, bounded rounds, a completeness critic and an optional soft budget.
- Each profile preloads the contracts of the functions its role calls on every run (orchestrator: kanban, state, harness spawn/status/stop/metrics and triggers; work tiers: `shell::exec`, `coder::read-file`, plus `coder::create-file` / `coder::update-file` for `ultracode-impl`; verify tiers: `state::get`, `shell::exec`).
- Lean context: the orchestrator starts without the skills index (`skills_index: false`), asks for compact kanban writes and context-free `harness::metrics`, and its leaves drop the index through their deny list. Harness, iii-directory and kanban builds without these options ignore them.
- Requires `harness` `^1.8.53`, `state` `>=0.22.18`, `ide` `>=0.12.0` and the first `kanban` release with `kanban::ticket::links` and `add_labels` (`^0.1.21`). Does not use the `workflow` worker.
