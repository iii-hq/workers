# llm-budget

Workspace and agent LLM spend caps with period rollover, threshold alerts, spend recording, forecasting, and per-principal exemptions.

- [`llm-budget`](iii://llm-budget)
  - [`budget::create`](iii://llm-budget/create) — create a budget with ceiling + period
  - [`budget::list`](iii://llm-budget/list) — list budgets, newest first
  - [`budget::get`](iii://llm-budget/get) — fetch a budget by ID
  - [`budget::update`](iii://llm-budget/update) — patch whitelisted budget fields
  - [`budget::delete`](iii://llm-budget/delete) — permanently delete a budget
  - [`budget::reset`](iii://llm-budget/reset) — reset spent_usd and archive the prior window

  - [`budget::check`](iii://llm-budget/check) — check whether a budget allows an estimated spend
  - [`budget::record`](iii://llm-budget/record) — record an actual spend and fire threshold alerts
  - [`budget::usage`](iii://llm-budget/usage) — aggregate spend over a window (current + archived)
  - [`budget::forecast`](iii://llm-budget/forecast) — project spend through period end

  - [`budget::alert_set`](iii://llm-budget/alert_set) — add a threshold alert with a callback
  - [`budget::enforce`](iii://llm-budget/enforce) — toggle enforcement on a budget
  - [`budget::exempt`](iii://llm-budget/exempt) — grant a principal a 24-hour exemption
  - [`budget::pause`](iii://llm-budget/pause) — pause or resume a budget

For cost-per-token data used to estimate LLM call costs before calling `budget::check`, see [`models-catalog`](iii://models-catalog).
