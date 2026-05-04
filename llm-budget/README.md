# llm-budget

Workspace + agent LLM spend caps on the iii bus. Registers `budget::*` for
budget CRUD, alerts, forecast, and period rollover.

## Installation

```bash
iii worker add llm-budget
```

## Run

```bash
iii-llm-budget --engine-url ws://127.0.0.1:49134
```

State is persisted via iii state — survives restart when paired with a
durable iii engine backend.

## Registered functions (14)

`budget::create`, `budget::get`, `budget::list`, `budget::update`,
`budget::delete`, `budget::record_spend`, `budget::check`, `budget::forecast`,
`budget::rollover`, `budget::alert_set`, `budget::alert_clear`,
`budget::exemption_grant`, `budget::exemption_revoke`, `budget::log_list`.

## Build

```bash
cargo build --release
```
