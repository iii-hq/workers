# mesh

Mesh joins iii engines. One project runs the hub. Other projects run the same
worker in client mode: they join the hub with its URL and a join code or token,
need no root, and share chosen local workers, which the hub project calls under
names such as `laptop::shell::exec`. It uses
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

## Quickstart

With the hub running, inspect its health and discover joined nodes:

```bash
iii call --engine ws://127.0.0.1:49834 mesh::status
iii call --engine ws://127.0.0.1:49834 mesh::nodes::list
iii call --engine ws://127.0.0.1:49834 mesh::nodes::get name=laptop
```

Status reports front-door, gateway and proxy health plus total, online and
outdated node counts. The list holds each node's name, address, platform,
version, online state and shared workers. An empty list means no client has
joined. An offline node exposes no functions.

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
This source is not a node grant. Local tests cover SDK delivery and control
transitions.

## Console

Open **Mesh** in the Console of a hub to see the hub address and the joined
nodes with their connectivity, connection path, version and shared workers.
Use **Refresh** for a new snapshot; the page also refreshes when you return to
the tab. The **Connect another engine** section shows this hub's URL and join
code, to paste into the Mesh page of the other project in **Client** mode. If
the public URL is missing, set **Public URL** or turn on **Expose through a
tunnel** in the page's **Hub address** section, then refresh the join
instructions.

The settings action opens the hub's effective configuration entry, including
an `III_CONFIG_NAME` override. Console owns validation, unsaved changes, save
and reset. Network settings and trusted proxies are editable there.

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
```

Set `public_url` to a reachable front door before a client joins. Put the
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
and enrollment state.

Entries written before 2026-11-10 may hold `links`, `nodes` and `drives`. The
hub accepts and ignores them, and nothing removes them.

## Expose through a tunnel

Without a front door of your own, the hub can borrow a public address from the
quick-tunnel worker. Turn on "Expose through a tunnel" on the Mesh page, or set:

```yaml
tunnel:
  enabled: true
  tunnel_id: mesh      # optional, defaults to mesh
```

List the hub's `listen` port as a target in the quick-tunnel configuration;
mesh never edits another worker's config. `targets` replaces quick-tunnel's
default list, so keep `webhooks` for github:

```yaml
provider: opentunnel
targets:
  webhooks: http://127.0.0.1:3112
  mesh: http://127.0.0.1:49180
proxy_protocol: [mesh]
```

The public URL is `public_url` when set, otherwise the tunnel URL once the
tunnel is `ready`, otherwise empty. While `public_url` is set the hub holds no
lease and `mesh::status` reports the tunnel as `overridden`. The hub holds a
10-minute lease, renews it when under 5 minutes remain and releases it on
shutdown, so quick-tunnel's `max_lease_seconds` must be at least 600. A failed
quick-tunnel call keeps the last URL while the lease is unexpired.
`mesh::status` reports `tunnel.state` (`starting`, `ready`,
`reconnecting`, `failed`, `stopped`, `unavailable` or `overridden`),
`tunnel.error` and `tunnel.proxy_protocol` (`v2` while the hub reads visitor
addresses from the tunnel). A tunnel carries no UDP, so STUN still requires `public_url`.

- **Use `provider: opentunnel`.** Its URL is stable. With cloudflared the URL
  changes whenever the tunnel process restarts, including every hub restart.
  Clients already joined and tokens already issued keep the old URL and stop
  working; the hub does not notify them.
- **The URL is hard to guess, not secret.** It is
  `https://<route>.<id>.opentunnel.xyz`. Certificate Transparency logs list
  `<id>.opentunnel.xyz` as soon as its certificate is issued, and scanners
  read them, but that name does not answer. `<route>` is a random name kept by
  quick-tunnel that is in no certificate. DNS resolvers, the relay, everyone
  holding the hub URL, and every local worker or agent allowed to call
  `mesh::status` or `quick-tunnel::status` still see it, so join codes and
  tokens remain the only protection.
- **List the target in `proxy_protocol`.** quick-tunnel then starts every
  tunnel connection with a PROXY protocol v2 header that carries the visitor's
  address, and the join rate limit (10 attempts per 10 minutes) counts each
  visitor separately; an IPv6 visitor is counted by its /64. `mesh::status`
  shows `tunnel.proxy_protocol: v2` when it is in use. Without it every
  visitor reaches the hub as `127.0.0.1` and they share one bucket: anyone who
  sends 10 junk `POST /join` requests every 10 minutes blocks every join
  through the tunnel, join codes and tokens alike. List only the target that
  points at this hub's `listen` port: quick-tunnel checks that targets sharing
  a port agree, not that the port is the hub's.
- The hub reads that header only from loopback, and only while quick-tunnel
  lists the hub's lease with `proxy_protocol: v2`. If quick-tunnel stops
  answering, the hub stops reading it and tunnel visitors get `400` until it
  answers again. While it is on, any local process that connects to `listen`
  can state a visitor address, and so can any other tool that forwards raw
  connections to that port (an SSH forward, a container port mapping that
  makes outside peers look like loopback); the address itself is the one the
  OpenTunnel relay reports. A mesh worker older than this feature answers
  `400` to every tunnel visitor when its target is listed.
- **`trusted_proxies` and the provider.** With cloudflared set
  `trusted_proxies: [127.0.0.1/32, ::1/128]`: cloudflared connects from
  loopback and Cloudflare sets `CF-Connecting-IP`; `proxy_protocol` does not
  exist there. With opentunnel a forwarding header is written by the visitor,
  so while the tunnel runs on opentunnel the hub drops `CF-Connecting-IP` and
  `X-Forwarded-For` from loopback peers and a loopback entry in
  `trusted_proxies` has no effect. Remove it anyway when you switch provider:
  a mesh worker older than this feature still honors it.
- Only the front door (`listen`) is exposed, the same surface `public_url`
  already publishes. The loopback admission listener is not routed.

## Join another engine

Call `mesh::join::code` on the hub. It returns `{"code":"XXXX-XXXX"}`; the code
is a credential and this call requires approval:

```bash
iii call --engine ws://127.0.0.1:49834 mesh::join::code
```

On the other machine, run the mesh worker in [client mode](#client-mode) with
`client.hub_url` set to this hub's public URL (`mesh::status`, field
`public_url`) and the code stored as the `client.join` secret. The hub's Mesh
page shows both under **Connect another engine**. `mesh::join::cycle` replaces
the code. `mesh::nodes::remove name=<node>` removes a node, revokes its token
and cycles the code.

## Node boundary

Every node connection passes the hub gateway and rbac-proxy. Its functions
register under the node's name, and `<node>::mesh-node::*` accepts only
requests signed by the hub. A node gets no call grants in the project's
namespace. In `default` it may call `configuration::get`, `ensure`, `register`
and `set` only for entries named `<node>__*` (reads are raw, templates are
refused) and observe `configuration::on-updated` for those entries. It may also
observe its own `<node>::*` functions in the project's namespace, with empty
args. Other observations, ADE assets and condition callbacks are refused. A
client bridge only receives calls and uses none of these grants.

## Isolated local build and validation

Use iii 0.26.0 or a compatible 0.26 build with the pinned SDK
`v0.26.0-rc.5`. Install the repository-pinned Go, Rust, Node and pnpm versions;
`mesh/iii.worker.yaml` declares the `>=0.26.0` engine floor. Build current
frontends and auxiliary workers before running the complete suite:

```sh
pnpm --filter '@iii-workers/harness-ui...' --filter '@iii-workers/state-ui...' --filter '@iii-workers/mesh-ui...' install --frozen-lockfile
for directory in harness/ui state/ui mesh/ui; do
  pnpm --dir "$directory" build
done
for worker in rbac-proxy harness fp state queue session-manager; do
  cargo build --locked --manifest-path "$worker/Cargo.toml" --bin "$worker"
done
III_BIN=/absolute/path/to/iii mesh/tests/e2e/run-tests.sh
node --test mesh/tests/e2e/public-report.test.mjs
```

The runner allocates loopback engine and proxy ports and temporary state, and
cleans up only its own process groups. Raw logs/frames can contain credentials:
keep `MESH_E2E_REPORT_DIR` private (the runner sets directory mode 0700 and
files use 0600). Publish only `public-report.json` from the allowlisted
projection. The runner starts a real engine, rbac-proxy and fp, plus harness,
state, queue and session-manager for the hook test, with an in-process hub. It
checks the boundary a node connection meets and does not run a client-mode
engine. ADE source-contract fixtures prove Mesh asset registration/content; the
browser check uses controlled bus data with the actual ADE SPA loader.

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
Live publication and cross-network operation remain unverified.

## Unattended enrollment

Issue a single-use token with an optional lifetime in seconds:

```bash
iii call --engine ws://127.0.0.1:49834 mesh::join::token expires_in_secs=3600
```

The response contains `token_id` and `token`. Store the token as the client's
`client.join` secret; a token carries the hub URL, so `client.hub_url` is not
needed. The `iiimj_` token contains the exact hub URL, its identity pin and a
random credential. Keep it private. The hub stores only the credential hash;
the client saves its credential and pinned identity in
`data/mesh-client/join.json`.

The first successful durable registration consumes a single-use token. A failed
hub state save leaves it usable. Omitted or zero `expires_in_secs` means no
expiry; negative values are refused. To revoke future enrollment:

```bash
iii call --engine ws://127.0.0.1:49834 mesh::join::revoke token_id=<returned-id>
```

Revocation and expiry leave the join code and already accepted nodes intact.
Removing a node also revokes its creating token, including reusable tokens,
and cycles the join code. Reusable tokens require ephemeral nodes:

```bash
iii call --engine ws://127.0.0.1:49834 mesh::join::token ephemeral=true reusable=true expires_in_secs=3600
```

An ephemeral node disappears after three minutes without map activity. Healthy
open map streams remain active; their progress is persisted so a hub restart
preserves the remaining idle interval. Expiry updates the hub's node list and
revokes the node's sessions. Local tests cover issuance, enrollment, restart and
removal.

## Client mode

A client is a project's own mesh worker joined to another project's hub as a
node, without root, publishing chosen local workers there. The hub sees them
as `<node>::<function>`, for example `harness-mbp::mesh-probe::echo`, and
each call is forwarded to the local engine. Networking runs in userspace: no
TUN device, no root, no system service. A client opens no hub listener and
writes no hub state.

```yaml
mode: client                          # absent means hub
client:
  hub_url: https://mesh.example.com   # needed with a join code; a token carries its URL
  join: secret://MESH_JOIN            # secret://NAME or env://NAME, never the code itself
  name: harness-mbp                   # requested; the hub may assign harness-mbp-2
  share: [mesh-probe]                 # local worker names; [] shares nothing
```

Store the hub's join code or `iiimj_` token as secret `MESH_JOIN` with
consumers `[mesh]`; the Console's mesh form does this. The value is used once to
enroll. State lives in `$III_COMPOSE_DIR/data/mesh-client` (0700, `join.json`
0600) and later boots reuse its node key. The hub allows 10 join attempts per
address per 10 minutes, so a join the hub refused waits until the join value
changes; an unreachable hub or a missing `hub_url` is retried. `join.json` is
written only after the hub accepts the node.
After the hub removes the node, delete `data/mesh-client` to rejoin. Seed the
mode from compose so the first boot is already a client:

```yaml
  mesh:
    worker: path://../../mesh
    start_after: [secrets]
    config_override: { mode: client }   # override only mode; the Console edits client.*
    scripts: { pre_run: make build, run: ./bin/mesh }
```

`share` changes apply within about five seconds; other changes need a restart.
`mesh::status` and the Console's Mesh page report the hub, assigned name, hub
namespace, state and each shared worker with its hub function IDs.

Anyone in the hub project can call shared workers, and calls reach the local
engine as caller `mesh`. Sharing `ide`, `harness` or any worker that runs
commands gives the hub project remote code execution on this machine. `mesh`,
`secrets`, `configuration` and `rbac-proxy` cannot be shared, and functions
under `engine::`, `mesh::`, `mesh-node::`, `secrets::`, `configuration::` and
`rbac-proxy::` are never published. Inbound tailnet connections to the client
are refused and the local `III_TOKEN` is not sent to the hub. The hub gateway is
reached through a random loopback port, so any local process that reaches it
acts as this node toward the hub.

The hub hands its namespace only to a node that runs its exact version, and
nothing updates itself. On a mismatch the client's status shows the version the
hub wants and a client that starts shares nothing; a client that was already
sharing when the hub restarted on another version keeps its registrations until
it restarts. Build with `make build`, which stamps the `package.json` version (a
plain `go build` has none and fails), and bump clients with the hub.

Bridging is request/response only. Channel refs in a payload or result are
refused with `channels_unsupported`; triggers, observers and events are not
mirrored; local calls time out after 10 minutes and a hub caller's cancellation
does not reach them. Only whole workers can be shared and local workers cannot
call hub functions. Windows clients and payloads over 16 MiB are unsupported.
Live acceptance against a running hub remains unverified.

## State and removal

The hub keeps its identity and enrollment state in
`$III_COMPOSE_DIR/data/mesh/state.json`. A client keeps its node key, credential
and pinned hub identity in `$III_COMPOSE_DIR/data/mesh-client` (0700,
`join.json` 0600). After `mesh::nodes::remove`, the client stops and reports
`removed by hub; delete data/mesh-client to rejoin`.

State written before 2026-11-10 may hold machine nodes. They stay listed and can
still connect. On the hub's version the hub hands them an empty worker set, so
they stop the workers they run; on another version it counts them as outdated
and pushes no update. The hub sends no drive calls and drive peers stop seeing
each other, so existing mounts stop answering. To retire such a machine, first
call `mesh::nodes::remove` while its old service still runs: the hub refuses its
keys, revokes its token and cycles the join code, and the service parks. Then
run the old binary's `sudo mesh leave` on the machine to remove its service,
mounts and state. In the reverse order the leave logs the node out, the hub
deletes its record, and `mesh::nodes::remove` fails with `no removable node`
without revoking the token or cycling the code.

Upgrade the hub before its clients, or both together: a hub of the earlier
build shares nothing with a client of this one.

## Agent access

The [Your machines skill](skills/SKILL.md) describes machine discovery and
remote paths. Discovery reads are allowed; remote shell/coder calls require
approval by default, even when the local equivalent is allowed. Node control
(`mesh-node`), proxy, configuration-hook and console internals are denied to
agents.

See [NOTICE](NOTICE) for upstream provenance and licenses.

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
`pnpm --dir mesh/ui test` from the repository root (it builds the page first). The check starts an
isolated local Console with controlled bus fixtures and writes screenshots to
`/tmp/mesh-task22-render` (override `MESH_UI_ARTIFACTS`). It does not verify a
real hub or tunnel.
