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

## Console

Open **Mesh** in the Console to see machine connectivity, connection path,
version, workers and shared-drive state. Use **Refresh** for a new snapshot;
the page also refreshes when you return to the tab. The join section shows the
current device code and copyable Linux/macOS and Windows commands. If the
public URL is missing, configure it through the page's **Configure worker**
action, then refresh the join instructions.

The settings action opens the hub's effective configuration entry, including
an `III_CONFIG_NAME` override. Console owns validation, unsaved changes, save
and reset. Machine worker lists, additional grants, drive servers and clients,
network links and trusted proxies are editable there.

Node worker identity discovery was checked against a real engine and proxy in
both `default` and project namespaces: `laptop::ide::configuration-id` resolves
to `laptop__ide`, whose form family is `ide`. Remote IDE UI asset registrations
are currently rejected by Mesh's trigger policy. A Console with an already
loaded IDE form can reuse that family; node-only form delivery needs follow-up.
The exact configuration entry remains accessible through
`configuration::get` / `configuration::set` with ID `<node>__<worker>`.

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
drives:
  proj:
    server: linuxbox                 # joined Linux node with apt
    nodes: ["*"]                    # every persistent node; or explicit names
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

Have an operator call `iii trigger mesh::join::code` and run the returned platform
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
Linux systemd, macOS launchd, and Windows service installation are implemented.
On Windows 11 (amd64 or arm64), run the returned PowerShell command as the
intended operator:

```powershell
& ([scriptblock]::Create((irm https://raw.githubusercontent.com/iii-hq/workers/mesh/v<version>/mesh/install/install.ps1))) <version> <public_url> XXXX-XXXX
```

The PowerShell installer verifies the exact registry artifact before running
`mesh join`, which requests UAC elevation and preserves the original caller's
SID even when another administrator supplies credentials. Windows runs workers
in that saved operator's existing logon session, preferring an active session.
An absent operator produces a worker error; it never selects another account.
Use `& "$env:ProgramFiles\iii-mesh\mesh.exe" status` after installation.
Windows installation, UAC, session execution, TUN/routes/hosts and direct/relayed
network acceptance remain unverified; local tests and cross-builds cover the
implemented boundaries.

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

Linux and macOS install the executable at `/usr/local/bin/mesh`. Worker binaries
are verified in a separate public cache so operator-run workers can execute
them without reading private enrollment credentials.

| Location | Linux | macOS | Windows |
| --- | --- | --- | --- |
| Service | `/etc/systemd/system/iii-mesh.service` | `/Library/LaunchDaemons/dev.iii.mesh.plist` | SCM service `iii-mesh` (LocalSystem) |
| Private state | `/var/lib/iii-mesh` | `/Library/Application Support/iii-mesh` | `%ProgramData%\iii-mesh` |
| Worker cache | `/var/cache/iii-mesh/workers` | `/Library/Caches/iii-mesh/workers` | `%ProgramData%\iii-mesh-workers` |
| Service logs | systemd journal (`iii-mesh.service`) | `node.log` under private state | `node.log` under private state |

Windows installs `mesh.exe` and pinned Wintun 0.14.1 beside each other under
`%ProgramFiles%\iii-mesh`. The known system folders are resolved through the
Windows API. Private state has a protected ACL granting only SYSTEM and
Administrators full access. The binary directory and worker cache grant those
accounts full access and Builtin Users read/execute access. Cache entries are
`<sha256>.exe`; temporary downloads are admin-only until verified. Existing
unsafe owners, ACLs, and reparse paths are refused. Wintun's archive and both
architecture DLL digests are pinned, and the DLL is verified before TUN creation.

Private state contains `join.json`, backend keys under `mesh/`, worker logs
under `logs/`, and lifecycle markers. `mesh status` reads the local
`/var/run/iii-mesh.sock` socket, or `\\.\pipe\iii-mesh` on Windows. After hub removal, the service closes workers
and networking, records removal and parks with a read-only status listener.
It stays inert across restarts until explicit `mesh leave`; it does not rejoin.

Leave removes credentials, backend state, worker cache and installed executable
only after acknowledged logout or confirmed hub removal. A failed or missing
logout acknowledgement returns an error and retains the material needed to
retry, even though the service may already be unregistered. Restore hub
reachability, run `sudo mesh node` to finish the pending logout, then retry
`mesh leave`. Do not delete the retained enrollment state to bypass this step.
On Windows, run the installed `mesh.exe node` from an elevated terminal for a
pending logout retry. Windows may schedule deletion of a loaded executable for
the next reboot. Native service operation and cleanup on fresh hosts remain
unverified on all platforms.

## Agent access

The [Your machines skill](skills/SKILL.md) describes machine discovery, remote
paths and the shared-drive mount and locking contract. Discovery reads are
allowed; remote shell/coder calls require approval by default, even when the
local equivalent is allowed. Supervisor, proxy, configuration-hook and console
internals are denied to agents. An operator changes the desired worker list
through configuration when a machine lacks tools.

Shared drives and the Console page are implemented. A drive server
requires Linux with apt and no existing NFS exports or listener on port 445.
Its first drive installs NFS and Samba. Data stays in `/srv/iii-mesh/<drive>`
and belongs to that server's operator; exports permit only member addresses.
The dedicated `iii-mesh-smbd` service admits only the mesh IPv6 /48 and uses
SMB3. Drive membership adds connectivity between the server and each member.

Linux mounts use `/drives/<drive>` over NFSv4; macOS uses `/Volumes/<drive>`
over NFSv4.0. Windows assigns available letters from `W:` downward and runs
mapping commands in the saved operator's authenticated session. That operator
must be logged in. Wildcards exclude ephemeral nodes; name them explicitly to
grant access. Use fcntl (`lockf`) for shared locks, because local `flock` locks
on the server do not coordinate with NFS clients.

An unreachable server leaves file operations waiting. Normal service restarts
and updates preserve mounts. Removing membership or leaving the mesh unmounts
owned drives; a server with no drives stops its exports and dedicated Samba
service. Packages and `/srv/iii-mesh` data remain. Cleanup failures retain
ownership state for retry and prevent successful leave acknowledgment. Local
tests cover signed reconciliation and injected OS effects; native mount,
service, packet-confinement and cross-machine locking acceptance remains
unverified on all platforms. See [NOTICE](NOTICE) for upstream provenance and
licenses.

## Building the Console assets

The Go binary embeds committed `assets/page.js` and `assets/styles.css` through
its top-level assets package. A fresh `go vet`, `go test` or `make build` needs
no Node installation or frontend output. After changing `ui/`, use the pinned
repository Node/pnpm toolchain and rebuild before committing:

```sh
pnpm --filter '@iii-workers/mesh-ui...' install --frozen-lockfile
make -C mesh ui
```

The UI uses the shared Console build driver, workspace catalog and lockfile.
Mesh E2E CI rebuilds the frontend and checks that committed bundles match.
Release builds use the embedded bundles without installing frontend packages.
For browser checks, install the Console SPA's dependencies too, then run
`node mesh/ui/tests/render.mjs` from the repository root. The check starts an
isolated local Console with controlled bus fixtures and writes screenshots to
`/tmp/mesh-task22-render` (override `MESH_UI_ARTIFACTS`). It does not verify
native machine installation, tunnels or mounts.
