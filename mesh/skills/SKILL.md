---
name: your-machines
description: Use when an agent needs to discover mesh nodes or call workers that another machine shares with this hub.
---

# Your machines

Mesh exposes the workers each online machine shares under that machine's name.
Remote paths and configured roots belong to that machine.

## Discovery and calls

| Need | Function and input |
| --- | --- |
| Find machines | `mesh::nodes::list` with `{}` |
| Inspect a machine | `mesh::nodes::get` with `{"name":"laptop"}` |
| Run a command on laptop | `laptop::shell::exec`, same input as `shell::exec` |
| Read a file on laptop | `laptop::coder::read-file`, same input as `coder::read-file` |

For example, to read `src/app.rs` on laptop, use
`laptop::coder::read-file` with the usual `coder::read-file` input for that
path. Resolve it under laptop's configured roots, using laptop's filesystem;
the hub's paths and roots do not apply. Retrieve the usual tool's schema when
its input fields are unknown.

An offline machine has **no functions**. A machine exposes only the workers its
owner shares; a missing tool means that worker is not shared.

Discovery reads are allowed. Remote calls need approval by default; a local
tool's allow rule does not grant access to its remote counterpart.

## Current call and observation API

Use `Call` / `call` for RPCs. For CLI discovery, select the intended engine
explicitly, for example:

```sh
iii call --engine ws://127.0.0.1:49834 -n project mesh::nodes::list --json '{}'
```

This example selects an isolated local engine. Match the namespace
to the hub. Build and isolation instructions are in [the Mesh README](../README.md#isolated-local-build-and-validation).

Hub integrations observe `mesh::on-node-changed` using the SDK's `Observe`,
with optional args `{"node":"laptop"}`. Register a callback, wait for observer
readiness, and dispose it when finished. This private producer is unavailable
as a direct tool call and is not a default node grant. Use machine discovery
for the current snapshot; observations contain no queued/unadmitted history.
An already-admitted SDK callback may enqueue after reconnect to the same
retained observer. Prefer generated observer IDs; explicit IDs are global.

## Node control boundary

Never call `*::mesh-node::*`, including `laptop::mesh-node::workers::apply`.
These are signed hub-to-node control calls, denied to agents. A missing tool is
fixed by the machine's owner sharing that worker in their Mesh page.
