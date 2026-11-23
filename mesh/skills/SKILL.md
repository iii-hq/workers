---
name: your-machines
description: Use when an agent needs to discover mesh machines or call workers that the hub or another machine shares in the mesh.
---

# Your machines

Mesh exposes the workers each machine shares. A machine name is a namespace:
a shared function keeps its id and lives in the namespace named after its
machine. On the hub, an online machine's shared functions are in that machine's
namespace. On a joined machine, the hub's shared functions are in namespace
`hub` and another machine's in the namespace named after it. Call a function
with its id and its namespace. A call runs on the machine that shares the
function, so its paths and configured roots belong to that machine.

## Discovery and calls

| Need | Function and input |
| --- | --- |
| Find machines, on the hub | `mesh::nodes::list` with `{}` |
| Inspect a machine, on the hub | `mesh::nodes::get` with `{"name":"laptop"}` |
| Run a command on laptop, from the hub or another machine | function `shell::exec`, namespace `laptop` |
| Read a file on laptop, from the hub or another machine | function `coder::read-file`, namespace `laptop` |
| Find what you can call, on a joined machine | `mesh::status` with `{}`, field `remote` (the hub first, then each machine with its function ids; its name is the namespace) |
| Run a command on the hub, from a machine | function `shell::exec`, namespace `hub` |

For example, to read `src/app.rs` on laptop, call `coder::read-file` in
namespace `laptop`, from the hub or from another machine, with the usual input
for that path. Resolve it under laptop's configured roots, using laptop's
filesystem; the caller's paths and roots do not apply. In namespace `hub` the
hub's filesystem and roots apply.
Retrieve the usual tool's schema when its input fields are unknown.

An offline machine has **no functions**. A machine exposes only the workers and
single functions its owner shares; a missing tool means it is not shared. A
function shared alone may be internal: it is not in the default function list
or tool catalog, but calling its id in the owner's namespace works. A machine's
own functions stay in its own namespace. A call in namespace `hub` or another
machine's namespace fails with `mesh_offline` while this machine is not connected or
the hub, just restarted, has not read it yet, `FORBIDDEN` when the function is
no longer shared, and `mesh_busy` when this machine already has 64 calls in
flight. A call that ended with `mesh_offline` or a timeout may still have run
on its target: check before retrying one that changes something.

`mesh::status` `remote[].path` says how each machine is reached: `hub`, or
`direct`, `relay` or `idle` when both machines turned on peer to peer and the
call skips the hub. A call to such a machine is never retried through
the hub: it fails with `mesh_offline` when that machine cannot be reached, when
it leaves the mesh or is renamed during the call, or when either machine has not
reached the hub for 10 minutes. It fails with `mesh_busy` when the target has 64
calls in flight from this machine. A target that goes silent is reported only
when the call times out, so a call can wait up to about 10 minutes.

Discovery reads are allowed. Remote calls follow the same approval rules as
local ones, per namespace: a bare rule matches every namespace, so the
read-only allows, such as `coder::read-file`, run with no prompt on every
machine, and other remote calls need approval as they do locally. The machine
that runs a remote call does not ask again.

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

Never call `mesh-node::*` in any namespace, including `mesh-node::workers::apply`
in namespace `laptop`. These are signed hub-to-node control calls, denied to
agents. A missing tool is
fixed by the machine's owner sharing that worker in their Mesh page.
