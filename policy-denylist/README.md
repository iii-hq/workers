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

Without `POLICY_DENIED_TOOLS`, the worker uses a small built-in default
(`bash:rm -rf,sudo,curl-pipe-bash`).

## Registered functions

| Function | Description |
|---|---|
| `policy::denylist` | Subscriber bound to `agent::before_tool_call`. Replies `{ block: true, reason }` for matches; `{ block: false }` otherwise. |

## Runtime expectations

The worker subscribes to `agent::before_tool_call`. That topic is
published by `harness-runtime` while the agent loop is executing — for
the denylist to fire, `harness-runtime` (or any worker emitting the same
topic) must be running on the bus.

## Build

```bash
cargo build --release
```
