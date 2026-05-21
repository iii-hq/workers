# llm-budget

Workspace + agent LLM spend caps with alerts, forecast, and period
rollover under `budget::*`.

## Purpose

Budgets cap how much an agent or workspace may spend on LLM tokens within
a rolling period. The worker exposes CRUD + `check`/`record` for the hot
path, an alerts surface (`alert_set` adds a threshold-percent alert that
fires once per period when crossed), a forecast that projects spend
through `period_resets_at`, and period rollover that archives the prior
period into `spend_log:<id>:<period_start>` before resetting `spent_usd`.

Backed entirely by iii state under the `budgets` scope. No durable
triggers — callers invoke `budget::check` before issuing a model call and
`budget::record` after.

## Registered functions

- `budget::create` — Create a budget with ceiling + period.
- `budget::list` — List budgets, newest first.
- `budget::get` — Fetch a budget by id.
- `budget::update` — Update a whitelisted set of budget fields.
- `budget::delete` — Delete a budget.
- `budget::check` — Check whether a budget allows an estimated spend.
- `budget::record` — Record a spend, fire matching alerts.
- `budget::reset` — Reset `spent_usd`, archive prior period.
- `budget::alert_set` — Add an alert to a budget.
- `budget::usage` — Aggregate spend over a window.
- `budget::forecast` — Project spend through period end.
- `budget::enforce` — Toggle enforcement on a budget.
- `budget::exempt` — Add a 24h exemption for a principal.
- `budget::pause` — Pause or resume a budget.

## Triggers

None.

## State keys

All keys live under iii state scope `budgets`. From
[src/llm-budget/types.ts](harness-node/src/llm-budget/types.ts):

| Key shape | Value |
|---|---|
| `budget:<id>` | `Budget` record (workspace_id, agent_id, name, ceiling_usd, period, spent_usd, period_start_at, period_resets_at, enforced, paused, alerts[], exemptions[]). |
| `spend_log:<id>:<period_start>` | `SpendLogEntry` rollup written when a period rolls over. |
| `spend_log:<id>:<period_start>:reset-<ts>-<suffix>` | Reset log written by `budget::reset` so manual resets are auditable. |

## Configuration

The worker reads no top-level config keys; period defaults and rollover
math live in
[src/llm-budget/periods.ts](harness-node/src/llm-budget/periods.ts).

## Dependencies

From
[src/llm-budget/iii.worker.yaml](harness-node/src/llm-budget/iii.worker.yaml):
`iii-state ^0.11.0`.

## Source layout

| File | Purpose |
|---|---|
| [src/llm-budget/main.ts](harness-node/src/llm-budget/main.ts) | Binary entry point (`iii-llm-budget`). |
| [src/llm-budget/register.ts](harness-node/src/llm-budget/register.ts) | Registers all 14 `budget::*` handlers. |
| [src/llm-budget/types.ts](harness-node/src/llm-budget/types.ts) | `Budget`, `Alert`, `Exemption`, `SpendLogEntry`, plus the `budgetKey` / `spendLogKey` / `resetLogKey` helpers. |
| [src/llm-budget/store.ts](harness-node/src/llm-budget/store.ts) | State CRUD: `loadBudget`, `saveBudget`, `deleteBudgetRecord`, `listAllBudgets`, `saveSpendLog`, `listSpendLogs`. |
| [src/llm-budget/periods.ts](harness-node/src/llm-budget/periods.ts) | `periodStart` / `nextPeriodStart` for `day` / `week` / `month`. |
| [src/llm-budget/ops.ts](harness-node/src/llm-budget/ops.ts) | Higher-level ops shared across handlers (rollover, alert firing, exemption checks). |
| [src/llm-budget/handlers/*.ts](harness-node/src/llm-budget/handlers/) | One handler per registered function (`create.ts`, `list.ts`, `get.ts`, `update.ts`, `delete.ts`, `check.ts`, `record.ts`, `reset.ts`, `alert-set.ts`, `usage.ts`, `forecast.ts`, `enforce.ts`, `exempt.ts`, `pause.ts`). |
| [src/llm-budget/handlers/index.ts](harness-node/src/llm-budget/handlers/index.ts) | Re-exports the per-handler `register*` functions. |
| [src/llm-budget/iii.worker.yaml](harness-node/src/llm-budget/iii.worker.yaml) | Worker manifest. |
