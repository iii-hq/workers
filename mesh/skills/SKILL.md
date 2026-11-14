---
name: your-machines
description: Use when an agent needs to discover mesh machines or call workers that the hub or another machine shares in the mesh.
---

# Your machines

Mesh exposes the workers each machine shares. On the hub, an online machine's
shared workers appear under that machine's name. On a joined machine, the
hub's shared workers appear as `hub::<function>` and another machine's as
`peer::<machine>::<function>`. A call runs on the machine that shares the
function, so its paths and configured roots belong to that machine.

## Discovery and calls

| Need | Function and input |
| --- | --- |
| Find machines, on the hub | `mesh::nodes::list` with `{}` |
| Inspect a machine, on the hub | `mesh::nodes::get` with `{"name":"laptop"}` |
| Run a command on laptop, from the hub | `laptop::shell::exec`, same input as `shell::exec` |
| Read a file on laptop, from the hub | `laptop::coder::read-file`, same input as `coder::read-file` |
| Find what you can call, on a joined machine | `mesh::status` with `{}`, field `remote` (the hub first, then each machine with its function ids) |
| Run a command on laptop, from another machine | `peer::laptop::shell::exec`, same input as `shell::exec` |
| Run a command on the hub, from a machine | `hub::shell::exec`, same input as `shell::exec` |

For example, to read `src/app.rs` on laptop, use `laptop::coder::read-file`
on the hub, or `peer::laptop::coder::read-file` on another machine, with the
usual `coder::read-file` input for that path. Resolve it under laptop's
configured roots, using laptop's filesystem; the caller's paths and roots do
not apply. For a `hub::` function the hub's filesystem and roots apply.
Retrieve the usual tool's schema when its input fields are unknown.

An offline machine has **no functions**. A machine exposes only the workers and
single functions its owner shares; a missing tool means it is not shared. A
function shared alone may be internal: it is not in the default function list
or tool catalog, but calling its `hub::` or `peer::` id works. A machine's own
functions keep their local ids and never appear under `peer::`. A `hub::` or
`peer::` call fails with `mesh_offline` while this machine is not connected or
the hub, just restarted, has not read it yet, `FORBIDDEN` when the function is
no longer shared, and `mesh_busy` when this machine already has 64 calls in
flight. A call that ended with `mesh_offline` or a timeout may still have run
on its target: check before retrying one that changes something.

`mesh::status` `remote[].path` says how each machine is reached: `hub`, or
`direct`, `relay` or `idle` when both machines turned on peer to peer and the
call skips the hub. A `peer::` call to such a machine is never retried through
the hub: it fails with `mesh_offline` when that machine cannot be reached, when
it leaves the mesh or is renamed during the call, or when either machine has not
reached the hub for 10 minutes. It fails with `mesh_busy` when the target has 64
calls in flight from this machine. A target that goes silent is reported only
when the call times out, so a call can wait up to about 10 minutes.

Discovery reads are allowed. Remote calls need approval by default, `hub::` and
`peer::` calls included; a local tool's allow rule does not grant access to its
remote counterpart. The machine that runs a remote call does not ask again.

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
