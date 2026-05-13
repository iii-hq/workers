Workspace and agent LLM spend caps on the iii bus — `budget::*` covers CRUD, pre-call checks, spend recording with threshold alerts, usage rollups, forecasting, enforcement and exemptions, and pause/resume. State is persisted via the engine's `state::*` helpers so budgets survive restarts when the storage backend is durable.

<!-- llm-only:start -->
Always pair `budget::check` (read with side-effects: rollover, exemption pruning) with a follow-up `budget::record` once the actual cost is known. `check` does not record spend, and `record` does not enforce the ceiling — calling only one of them silently breaks either the cap or the spend ledger.
<!-- llm-only:end -->
