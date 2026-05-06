# sandbox-host

A pseudo-sandbox worker that satisfies the `sandbox::*` surface used by
`shell-filesystem` and `shell-bash` by routing every call to **direct host
operations**. NO isolation, NO chroot, NO path scoping.

This exists so the harness can run shell tools on a solo-developer local
install without running a real sandbox stack. Multi-tenant deployments must
swap this out for a real isolated sandbox.

## Functions

| Function | Behavior |
|---|---|
| `sandbox::create` | Returns the fixed sandbox id `host`. |
| `sandbox::list` | Returns one entry: `[{sandbox_id: "host"}]`. |
| `sandbox::stop` | No-op. |
| `sandbox::fs::ls` | `std::fs::read_dir(path)`. |
| `sandbox::fs::read` | `std::fs::read(path)` returned via a `StreamChannelRef`. |
| `sandbox::fs::write` | Drains a `StreamChannelRef`, writes to disk. |
| `sandbox::fs::mkdir` | `std::fs::create_dir_all(path)` (or `create_dir` if `parents=false`). |
| `sandbox::fs::stat` | `std::fs::metadata(path)`. |
| `sandbox::exec` | Spawns the requested command directly on the host. |

## Trust

Tools call this directly with caller-supplied paths and commands. There is no
allowlist. Use only for single-tenant local installs.
