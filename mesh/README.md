# mesh

Mesh connects your machines to an iii project so their workers can be called
under names such as `laptop::shell::exec`. The hub coordinates private IPv6
networking, enrollment and worker updates; node services run workers as the
person who joined the machine. It uses
[configuration](https://github.com/iii-hq/workers/tree/main/configuration) and
[rbac-proxy](https://github.com/iii-hq/workers/tree/main/rbac-proxy).

## Install

```bash
iii trigger compose::add worker=mesh worker=rbac-proxy
```

Compose resolves dependencies and starts the hub in the project. Install or
update mesh and rbac-proxy together: `rbac-proxy::on-config-change` must return
`ok:true`, `applied_config`, `bound_host` and `bound_port`, with no error and a
live listener matching the mesh-owned configuration. Older proxies that return
only `{ok:true}` leave the mesh gateway inactive. These compatible changes
require coordinated publication before the registry install flow can be used.
The live `0.1.0` installer resolution returned HTTP 404 during local
verification.

## Quickstart

With the hub running, inspect its health and discover joined machines:

```bash
iii trigger mesh::status
iii trigger mesh::nodes::list
iii trigger mesh::nodes::get name=laptop
```

Status reports front-door, gateway and proxy health plus total, online and
outdated node counts. The list includes each machine's name, address, platform,
online state and worker state. An empty list means no machines have joined.
An offline machine exposes no functions. Windows has no `shell::*` or
`coder::*` until `ide` ships for Windows.

## Configuration

The hub creates the `mesh` entry in the configuration worker. Edit it through
the project's configuration UI; this is the equivalent value:

```yaml
public_url: https://mesh.example.com  # your HTTPS front door to the hub
listen: 0.0.0.0:49180                # admission, control and relay HTTP listener
trusted_proxies: []                  # CIDRs trusted to supply forwarded client IPs
gateway_port: 49135                 # private node-to-engine gateway
proxy_url: ws://127.0.0.1:49200      # direct loopback rbac-proxy endpoint
stun_port: 0                        # disabled; use a UDP port to enable STUN
links: []                           # explicit pairs of nodes allowed to connect
nodes:
  laptop:
    workers: [ide]                  # worker names, optionally name@version
    allow: []                       # extra hub function grants for this node
drives: {}                          # drive controllers are not available yet
```

Set `public_url` to a reachable front door before joining a machine. Put the
HTTP listener behind that front door. Configure the engine's worker listener
to bind to `127.0.0.1`: it defaults to `0.0.0.0`, and mesh does not change that
binding. Setting mesh's `III_URL` or `--url` only selects its connection
destination. Keep the proxy listener on loopback too. Proxy ownership requires
a direct `ws` numeric loopback address with an explicit port. Mesh refuses
foreign proxy ownership and verifies that the running proxy has applied the
mesh hooks and matching listener before opening the gateway. Failed application
revokes sessions and is retried automatically.
If an existing mesh-owned proxy entry disagrees with a changed engine URL or
proxy endpoint, update that entry to match; mesh does not overwrite it.

`III_URL` (or hub `--url`) selects the engine, defaulting to
`ws://127.0.0.1:49134`. `III_NAMESPACE` defaults to `default`;
`III_CONFIG_NAME` defaults to `mesh`. Hub state is stored at
`$III_COMPOSE_DIR/data/mesh`, or `data/mesh` relative to the working directory
when that variable is absent. Persist this directory to retain the hub identity
and enrollment state. Node worker configuration IDs are `<node>__<worker>`.

## Join a machine

Have an operator call `iii trigger mesh::join::code` and run the returned Unix
install command on the machine to join. The code is a credential and this call
requires approval. The versioned command has this form; substitute the returned
version, public URL and code:

```sh
curl -fsSL https://raw.githubusercontent.com/iii-hq/workers/mesh/v<version>/mesh/install/install.sh | sh -s -- <version> <public_url> XXXX-XXXX
```

Provision `curl`, `jq`, `sudo`, and `sha256sum` (Linux) or `shasum` (macOS)
first. In particular, a fresh macOS host needs `jq` installed separately.
The installer checks for `jq` before downloading the binary or elevating;
it does not install prerequisites. It resolves the exact hub version for the
machine's target, downloads over HTTPS and verifies SHA256 before sudo.
Registry artifacts and versioned installer URLs must be published first.
Fresh-host native installation and acceptance have not been verified.

Run as the intended non-root operator; join records that user through sudo.
Linux systemd and macOS launchd service installation are implemented. Windows
service installation and its PowerShell installer are not available yet.

| Command | Purpose |
| --- | --- |
| `mesh join <url> <code>` | Elevate, enroll, install and start the node service |
| `mesh join --port 41643 <url> <code>` | Choose the node UDP port (default 41643) |
| `mesh status` | Read local node and peer status without elevation |
| `mesh leave` | Elevate, request hub logout, stop/unregister the service and clean up after confirmation |
| `sudo mesh node` | Run the privileged service entrypoint, including a pending logout retry |

Token enrollment is not available yet. Workers run as the saved operator,
using `ws://hub.iii.internal:49135` and the hub namespace; their configuration
entry is `<node>__<worker>`. Self-updates restart through the service manager.

## Local state and removal

Both platforms install the executable at `/usr/local/bin/mesh`. Worker binaries
are verified in a separate public cache so operator-run workers can execute
them without reading private enrollment credentials.

| Location | Linux | macOS |
| --- | --- | --- |
| Service | `/etc/systemd/system/iii-mesh.service` | `/Library/LaunchDaemons/dev.iii.mesh.plist` |
| Private state | `/var/lib/iii-mesh` | `/Library/Application Support/iii-mesh` |
| Worker cache | `/var/cache/iii-mesh/workers` | `/Library/Caches/iii-mesh/workers` |
| Service logs | systemd journal (`iii-mesh.service`) | `node.log` under private state |

Private state contains `join.json`, backend keys under `mesh/`, worker logs
under `logs/`, and lifecycle markers. `mesh status` reads the local
`/var/run/iii-mesh.sock` socket. After hub removal, the service closes workers
and networking, records removal and parks with a read-only status listener.
It stays inert across restarts until explicit `mesh leave`; it does not rejoin.

Leave removes credentials, backend state, worker cache and installed executable
only after acknowledged logout or confirmed hub removal. A failed or missing
logout acknowledgement returns an error and retains the material needed to
retry, even though the service may already be unregistered. Restore hub
reachability, run `sudo mesh node` to finish the pending logout, then retry
`mesh leave`. Do not delete the retained enrollment state to bypass this step.
Native systemd/launchd operation and cleanup on fresh hosts remain unverified.

## Agent access

The [Your machines skill](skills/SKILL.md) describes machine discovery, remote
paths and the shared-drive mount and locking contract. Discovery reads are
allowed; remote shell/coder calls require approval by default, even when the
local equivalent is allowed. Supervisor, proxy, configuration-hook and console
internals are denied to agents. An operator changes the desired worker list
through configuration when a machine lacks tools.

Drive mounting/cleanup and the console page are future work; the skill's drive
paths describe the intended contract, not verified installed mounts. See
[NOTICE](NOTICE) for upstream source provenance and licenses.
