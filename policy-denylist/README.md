# policy-denylist

Hook subscriber on `agent::before_tool_call` that blocks any call whose
`tool_call.name` matches a configured denylist.

## Installation

```bash
iii worker add policy-denylist
```

## Run

```bash
POLICY_DENIED_TOOLS="bash:rm -rf,sudo" iii-policy-denylist
```

When started by the engine, the worker reads its `config:` block from
`--config <path>`. Defaults:

```yaml
topic: agent::before_tool_call
denied_tools:
  - "bash:rm -rf"
  - sudo
  - curl-pipe-bash
```

`POLICY_DENYLIST_TOPIC` and `POLICY_DENIED_TOOLS` override the config file for
direct runtime overrides.

## Registered functions

| Function | Description |
|---|---|
| `policy::denylist` | Subscriber bound to the configured topic. Replies `{ block: true, reason }` for matches; `{ block: false }` otherwise. |

## Runtime expectations

By default, the worker subscribes to `agent::before_tool_call`. That topic is
published by `provider-router` while the agent loop is executing — for
the denylist to fire, `provider-router` (or any worker emitting the same
topic) must be running on the bus.

## Build

```bash
cargo build --release
```
