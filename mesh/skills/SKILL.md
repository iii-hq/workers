---
name: your-machines
description: Use when an agent needs to discover mesh machines, call a remote machine's shell or coder functions, diagnose missing remote tools, or work on a shared mesh drive.
---

# Your machines

Mesh exposes each online machine's available tools under its name. Remote
paths and configured roots belong to that machine.

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

An offline machine has **no functions**. Inspect its discovery metadata and
wait for it to reconnect before calling its tools. A Windows machine has no
`shell::*` or `coder::*` until `ide` ships for Windows, even when online.

Discovery reads are allowed. Remote calls need approval by default; a local
tool's allow rule does not grant access to its remote counterpart.

## Shared drives

Use the mount on the machine running the operation:

| Machine OS | Drive mount |
| --- | --- |
| Linux | `/drives/<drive>` over NFSv4 |
| macOS | `/Volumes/<drive>` over NFSv4.0 |
| Windows | A drive letter allocated from `W:` downward, over SMB3 in the operator's session |

Discover the assigned Windows letter rather than assuming `W:` for every
drive. Coordinate shared edits with **fcntl (`lockf`) locks**. Never use
`flock` for cross-machine coordination: on the server it cannot see other
nodes' locks. File operations wait while the drive server is unreachable.

## Supervisor boundary

Never call `*::mesh-node::*`, including
`laptop::mesh-node::workers::apply`. These are signed hub-to-node supervisor
operations, denied to agents. Missing tools require the operator to inspect
the machine's worker configuration; they do not authorize a supervisor call.
