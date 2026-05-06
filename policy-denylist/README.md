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

## Workspace allowlist composition

Chat clients (e.g. `iii-console`) layer a workspace allowlist on top of `policy-denylist`. The allowlist enforces:

- Filesystem writes restricted to a configured workspace root.
- Absolute paths outside the workspace are rejected by the SDK wrapper *before* the bus call is dispatched.

`policy-denylist` remains the second layer (deny by tool name regardless of arguments). The two layers compose:

1. SDK wrapper (chat client side) — workspace allowlist on path arguments.
2. `policy-denylist` (engine side) — deny by tool name (exact match, case-sensitive).
3. `<ApprovalRow>` (chat UI) — per-call user approval surfaced inline before any write reaches disk.
