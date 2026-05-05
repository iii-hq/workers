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

`budget::create`, `budget::list`, `budget::get`, `budget::update`,
`budget::delete`, `budget::check`, `budget::record`, `budget::reset`,
`budget::alert_set`, `budget::usage`, `budget::forecast`, `budget::enforce`,
`budget::exempt`, `budget::pause`.

Function ids match `src/register.rs:18-33`; verify there before editing
this list.

## Build

```bash
cargo build --release
```
