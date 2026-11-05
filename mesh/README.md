# mesh

Mesh connects your machines to an iii project so their workers can be called
under names such as `laptop::shell::exec`. The hub coordinates private IPv6
networking, enrollment and worker updates; node services run workers as the
person who joined the machine. It uses
[configuration](https://github.com/iii-hq/workers/tree/main/configuration) and
[rbac-proxy](https://github.com/iii-hq/workers/tree/main/rbac-proxy).

## Install

```bash
iii call --engine ws://127.0.0.1:49834 compose::add worker=mesh worker=rbac-proxy
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
iii call --engine ws://127.0.0.1:49834 mesh::status
iii call --engine ws://127.0.0.1:49834 mesh::nodes::list
iii call --engine ws://127.0.0.1:49834 mesh::nodes::get name=laptop
```

Status reports front-door, gateway and proxy health plus total, online and
outdated node counts. The list includes each machine's name, address, platform,
online state and worker state. An empty list means no machines have joined.
An offline machine exposes no functions. Windows has no `shell::*` or
`coder::*` until `ide` ships for Windows.

## Node events

Observe the private `mesh::on-node-changed` source from a connected hub
client in the project's namespace. Its args accept only an optional node name:
`{}` watches all nodes and `{"node":"laptop"}` watches one. Unknown keys,
including condition callbacks, are refused. Register the callback first:

```go
_, err := iii.RegisterFunctionTyped(client, "audit::node-change", handler)
if err != nil { return err }
observer, err := client.Observe(iii.Source{
    FunctionID: "mesh::on-node-changed", Namespace: "my-project",
    Args: iii.Args{"node": "laptop"}, Deliver: iii.DeliverInput,
}, "audit::node-change", iii.ObserverOptions{TargetNamespace: "my-project"})
if err != nil { return err }
if err := observer.Ready(ctx); err != nil { return err }
defer observer.Dispose(ctx)
```

Direct remote calls to this private producer are refused; the hosting Mesh
worker publishes locally. Use SDK-generated observer IDs: explicit IDs are
global to the engine and reuse by another connection returns
`OBSERVER_ID_CONFLICT`.

The target receives `{"name":"laptop","event":"online","at":"2026-10-30T12:00:00Z"}`.
Events are `joined`, `online`, `offline`, `removed` and `outdated`. Connectivity
follows the node's active control streams. Removal includes logout and ephemeral
expiry. `outdated` is emitted when authenticated runtime metadata changes to a
version different from the hub, matching the status count. The hub itself is
excluded.

Delivery is best effort, with no event history or replay for new observers.
Restored inventory is a baseline; use `mesh::nodes::list` for a current snapshot.
The worker permits 256 active observers and 256 pending deliveries, drops overflow
and skips canceled or disconnected observers. A slow callback can delay other
callbacks for up to five seconds. Callback admission occurs after matching, under the delivery lock, using the
live binding and generation. Changed or removed generations fail that check
and drain before replacement acknowledgements. The SDK cannot atomically pin
an admitted local invocation to a connection epoch: a callback already admitted
can still enqueue after reconnection to the same retained observer. Queued or
unadmitted events are discarded across disconnects and never become history.
Node worker grants are unchanged; this source is not a default node grant.
Local tests cover SDK delivery and control
transitions; native machine, tunnel and mount acceptance remains unverified.

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
to `laptop__ide`, whose form family is `ide`. Remote IDE, browser, computer and
tmux ADE script/style/module asset observers are rejected by Mesh's observer
policy: an asset path prefix does not confine injected code or shared delivery.
A Console with an already loaded IDE form can reuse that family; node-only
form and page asset delivery remains unavailable.
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
    workers: [ide, browser, tmux]   # worker names, optionally name@version
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
The runtime upgrades only the known legacy Mesh-owned
`on_trigger_registration_function_id: mesh::on-trigger-registration` key to
`on_observer_registration_function_id: mesh::on-observer-registration` when
`middleware_function_id` is `mesh::middleware`. Operator fields and endpoints
are preserved; conflicting or foreign ownership remains refused.
If an existing mesh-owned proxy entry disagrees with a changed engine URL or
proxy endpoint, update that entry to match; mesh does not overwrite it.

`III_URL` (or hub `--url`) selects the engine, defaulting to
`ws://127.0.0.1:49134`. `III_NAMESPACE` defaults to `default`;
`III_CONFIG_NAME` defaults to `mesh`. Hub state is stored at
`$III_COMPOSE_DIR/data/mesh`, or `data/mesh` relative to the working directory
when that variable is absent. Persist this directory to retain the hub identity
and enrollment state. Node worker configuration IDs are `<node>__<worker>`.

## Join a machine

Have an operator call `iii call --engine ws://127.0.0.1:49834 mesh::join::code` and run the returned platform
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
| `mesh join --token <token>` | Enroll unattended with the token’s hub URL and identity pin |
| `mesh status` | Read local node and peer status without elevation |
| `mesh leave` | Elevate, request hub logout, stop/unregister the service and clean up after confirmation |
| `sudo mesh node` | Run the privileged service entrypoint, including a pending logout retry |

Workers run as the saved operator,
using `ws://hub.iii.internal:49135` and the hub namespace; their configuration
entry is `<node>__<worker>`. Self-updates restart through the service manager.


## Node workers and default grants

`ide`, `browser`, `computer` and `tmux` register functions under the node prefix,
for example `laptop::browser::sessions::start`. Local integration checks run the
actual binaries through signed node apply, the supervisor, a verified artifact
cache, the gateway and the real engine/proxy. IDE shell commands, headless
Chromium start/snapshot/stop, and tmux create/capture/kill on a private socket
are exercised. Install Chromium and tmux on the node for those operations.
The tmux worker forces UTF-8 output so a service's clean environment preserves
its structured command responses.

Configure each worker's owned entry, such as `laptop__browser`, before starting
it. Browser `executable`, `data_dir` and `headless` select its browser and
profile; tmux `tmux_binary` and `socket_path` select its server. Existing stored
values remain authoritative when a worker registers its schema.

The Linux supervisor does not pass `DISPLAY` or `XAUTHORITY` into workers.
Computer bootstrap and RPC registration work, but native screen operations
return a missing-display error in this environment. A separate local launch of
the same binary through the gateway captured an isolated Xvfb display; this
does not verify supervised desktop access or native machine acceptance.
Remote desktop endpoints are supported by the computer worker but were not
exercised in the Mesh integration check.

`DefaultAllow()` adds no shared grants. Effective node grants still include
owned configuration get/ensure/register/set calls in `default`, the
ownership-checked `configuration::on-updated` observer, and canonical same-node
function observations in the project's namespace. Additional operator `nodes.<name>.allow` values are preserved.
Configuration reads are raw and restricted to `<node>__*`; cross-node reads,
non-owned `shell-ui` reads and template expansion remain refused.

Observed optional requests do not become grants. The shared `state::*`,
`stream::*`, `queue::*`, `http::*`, `cron::*` and `harness::hook::*` surfaces
remain excluded from default grants. Computer session persistence/restore is unavailable because
its `state::list`, `state::set` and `state::delete` calls use shared scope
`computer_sessions`. Browser's harness guidance observer is refused, so its
automatic prompt injection is unavailable. ADE asset observers are also
refused; direct worker RPCs continue serving after those optional failures.
Same-node observations accept only empty args. Shared configuration observations
must select an owned ID with `"input.id":{"$eq":"<node>__<worker>"}` and may
select `"input.event_type":{"$eq":"configuration:updated"}`. Missing, broad,
malformed, foreign or condition-like filters/args are refused. Configuration
notifications deliver output and cause workers to re-fetch raw owned settings.

## Isolated local build and validation

Use iii 0.26.0 or a compatible 0.26 build with the pinned SDK
`v0.26.0-rc.5`. Install the repository-pinned Go, Rust, Node and pnpm versions;
`mesh/iii.worker.yaml` declares the `>=0.26.0` engine floor. Build current
frontends and auxiliary workers before running the complete suite:

```sh
pnpm --filter '@iii-workers/shell-ui...' --filter '@iii-workers/harness-ui...' --filter '@iii-workers/state-ui...' --filter '@iii-workers/mesh-ui...' --filter '@iii-workers/browser-ui...' --filter '@iii-workers/browser-native-ui...' --filter '@iii-workers/computer-ui...' --filter '@iii-workers/tmux-ui...' install --frozen-lockfile
for directory in ide/ui harness/ui state/ui mesh/ui browser/ui browser/native-ui computer/ui tmux/ui; do
  pnpm --dir "$directory" build
done
for worker in ide browser computer tmux rbac-proxy harness fp state queue session-manager; do
  cargo build --locked --manifest-path "$worker/Cargo.toml" --bin "$worker"
done
III_BIN=/absolute/path/to/iii MESH_TEST_CHROMIUM_BIN=/absolute/path/to/chromium mesh/tests/e2e/run-tests.sh
node --test mesh/tests/e2e/public-report.test.mjs
```

The runner requires Chromium, Xvfb and tmux. It allocates loopback engine/proxy
ports, temporary state, browser profiles, a display and tmux sockets, and cleans
up only its own process groups. Raw logs/frames can contain credentials: keep
`MESH_E2E_REPORT_DIR` private (the runner sets directory mode 0700 and files
use 0600). Publish only `public-report.json` from the allowlisted projection.
The runner uses controlled Registry resolve/cache fixtures, real signed apply,
supervised binaries and a real engine/proxy. ADE source-contract fixtures prove
Mesh asset registration/content; the browser check uses controlled bus data
with the actual ADE SPA loader.

For manual local inspection, start a separate loopback engine in its own
working directory. All CLI examples above explicitly target port 49834; ensure
these alternative ports are free. Product defaults remain unchanged.

```sh
repo=$(pwd)
lab=$(mktemp -d)
printf '%s\n' '{"workers":[{"name":"iii-worker-manager","config":{"host":"127.0.0.1","port":49834}}]}' > "$lab/engine.json"
(cd "$lab" && exec /absolute/path/to/iii --no-update-check --config "$lab/engine.json")
# In another terminal, using that same lab path:
# Build: (cd "$repo/mesh" && go build -o "$lab/mesh" .)
# Run: (cd "$lab" && exec ./mesh --url ws://127.0.0.1:49834)
iii call --engine ws://127.0.0.1:49834 -n project mesh::status --json '{}'
```

Configure Mesh's entry on that isolated engine before launching the hub:
`listen: 127.0.0.1:49880`, `public_url: http://127.0.0.1:49880`,
`proxy_url: ws://127.0.0.1:49871`, `gateway_port: 49835`, `stun_port: 0`.
Run a current rbac-proxy there too with a separate seed configuration pointing
to engine 49834 and listener `127.0.0.1:49871`. Set `III_NAMESPACE=project`
on both workers for project examples; keep `III_COMPOSE_DIR` under the lab.
Live publication, cross-network operation, native installs, supervised desktop
access and native networking/mount acceptance on Ubuntu/macOS/Windows remain
unverified.

## Unattended enrollment

Issue a single-use token with an optional lifetime in seconds:

```bash
iii call --engine ws://127.0.0.1:49834 mesh::join::token expires_in_secs=3600
mesh join --token <returned-token>
```

The response contains `token_id`, `token`, and executable Unix/Windows
`install` commands. Both installers accept tokens and check the downloaded
binary's SHA256 before elevation or join. The `iiimj_` token contains the exact
hub URL, its identity pin and a random credential. Keep it private. The hub
stores only the credential hash; the node saves its credential and pinned
identity in private enrollment state alongside its canonical operator account.

The first successful durable registration consumes a single-use token. A failed
hub state save leaves it usable. Omitted or zero `expires_in_secs` means no
expiry; negative values are refused. To revoke future enrollment:

```bash
iii call --engine ws://127.0.0.1:49834 mesh::join::revoke token_id=<returned-id>
```

Revocation and expiry leave the device code and already accepted nodes intact.
Removing a node also revokes its creating token, including reusable tokens,
and cycles the device code. Reusable tokens require ephemeral nodes:

```bash
iii call --engine ws://127.0.0.1:49834 mesh::join::token ephemeral=true reusable=true expires_in_secs=3600
```

An ephemeral node disappears after three minutes without map activity. Healthy
open map streams remain active; their progress is persisted so a hub restart
preserves the remaining idle interval. Ephemeral nodes are excluded from drive
`"*"` membership and must be named explicitly. Expiry updates peer maps, drive
membership and node revocation hooks. Local tests cover issuance, real Mesh
enrollment, restart, removal and both installer boundaries; native elevated
installation and published artifacts remain unverified.

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
