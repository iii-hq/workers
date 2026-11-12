# mesh

Mesh joins iii engines. One project runs the hub. Other projects run the same
worker in client mode: they join the hub with its URL and a join code or token,
need no root, and share chosen local workers, which the hub project calls under
names such as `laptop::shell::exec`. The hub can share its own workers too.
Every joined machine calls what the hub and the other machines share, under
names such as `hub::shell::exec` and `peer::laptop::shell::exec`. It uses
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
version, online state and shared workers; each shared worker lists its hub
function ids (`<node>::<function>`). The hub re-reads the online nodes when the
functions registered on its engine change, so a worker shared or stopped after
the join shows within about a second. What changes with no such event, a node's
connection path or a worker shared with no functions, shows within 30 seconds:
the hub also re-reads every online node at that interval. An empty list means
no client has joined. An offline node exposes no functions, though its workers
may still list the ids it last reported.

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
version different from the hub, matching the status count. A rename is reported
as `removed` under the old name, then `joined` and `online` under the new one.
The hub itself is excluded.

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

Open **Mesh** in the Console of a hub to see the joined machines, online first.
Each card lists the workers that machine shares, each with its hub function ids
and their descriptions, then its address, platform, version and connection path.
A card also warns when that machine shares more than the mesh carries.
**Remove** on a card removes that machine after a confirmation.
Next comes **Shared workers**: a switch for each of the hub's own workers,
which joined machines call as `hub::<function>`. A worker that runs commands is
marked. The **Hub address** section follows.
The page does not poll. The worker calls the private `mesh::on-status-changed`
with its `mesh::status` answer whenever that answer changes, and the page
observes it; in hub mode that covers the tunnel, an applied configuration, the
hub's shared workers and a machine's shared workers, and
`mesh::on-node-changed` covers the machines.
Use **Refresh** for a new snapshot; the page also refreshes when you return to
the tab. The **Connect another engine** section shows this hub's URL and join
code, to paste into the Mesh page of the other project in **Client** mode. If
the public URL is missing, set **Public URL** or turn on **Expose through a
tunnel** in the page's **Hub address** section, then refresh the join
instructions.

In **Client** mode the **Hub connection** section shows the client's state as
the worker pushes it (`starting`, `waiting for join`, `joining`,
`waiting for hub`, `connecting`, `connected`, `renaming`, `error`,
`join failed`, `removed by hub`, `left hub`), what happens next and whether the
worker keeps trying, and, while there is an enrollment to forget, **Leave
hub**. Both actions are described under [State and removal](#state-and-removal).
**Available from the mesh** lists what this machine can call, hub first: the
hub's functions as `hub::<function>` and each other machine's as
`peer::<machine>::<function>`, with their descriptions. It updates from the
same `mesh::on-status-changed` push.

In both modes a shared worker reads `not saved yet` only while the form differs
from the stored entry; once saved it reads `applying…` until the worker reports
it.

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
share: []                            # hub workers every joined machine may call
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

`share` lists the hub workers that every joined machine may call; it is empty
by default. As in `client.share`, `mesh`, `secrets`, `configuration` and
`rbac-proxy` are refused. A machine that joins or renames cannot take a shared
worker's name. `mesh::status` reports each entry under `shared`, with its
function ids as machines call them (`hub::<function>`) and a state: `shared`,
`not running`, `name conflict` (a joined machine has the name the worker's
function ids start with) or `too large`. `catalog_skipped` lists the machines
whose shares do not fit (see [Client mode](#client-mode)). A machine's call runs
with this project's access and nothing on the hub asks first, so sharing
`shell` or another worker that runs commands gives every joined machine remote
code execution on the hub.

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
page shows both under **Connect another engine**. The code works once: a
successful join replaces it, and the joined machine reconnects with its own
keys. `mesh::join::cycle` and the page's **Generate new code** button replace it
by hand. `mesh::nodes::remove name=<node>`, or **Remove** on the machine's
card, removes a node, revokes its token and replaces the code.

## Node boundary

Every node connection passes the hub gateway and rbac-proxy. Its functions
register under the node's name, and `<node>::mesh-node::*` accepts only
requests signed by the hub. A node's only call grant in the project's
namespace is `mesh::call`. No function registers that id: the hub's middleware
answers it by running a function from its list of shared functions. It refuses
with `FORBIDDEN` when the calling machine is not on the hub's version, the
function is not listed under the stated owner, the owner is the caller, a hub
worker is no longer in `share` or the call asks to be enqueued. Until it has
read the calling machine and what is shared, as after a restart, it answers
`mesh_offline`. A machine with 64 calls in flight gets `mesh_busy`, and
channel refs either way get `channels_unsupported`. The target runs the call
as caller `mesh`. Only the
hub knows the calling machine, and it logs each refusal with the machine, the
function and the owner, never the payload. In `default` a node may call
`configuration::get`, `ensure`, `register` and `set` only for entries named
`<node>__*` (reads are raw, templates are refused) and observe
`configuration::on-updated` for those entries. It may also
observe its own `<node>::*` functions in the project's namespace, with empty
args. Other observations, ADE assets and condition callbacks are refused. A
client bridge receives calls and calls only `mesh::call`; it uses none of the
configuration grants.

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

A token join, revocation and expiry leave the join code and already accepted
nodes intact; only a join with the code replaces it. Removing a node also
revokes its creating token, including reusable tokens, and replaces the join
code. Reusable tokens require ephemeral nodes:

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
each call is forwarded to the local engine. Every other joined machine calls
them through the hub as `peer::<node>::<function>`. Networking runs in
userspace: no TUN device, no root, no system service. A client opens no hub
listener and writes no hub state.

```yaml
mode: client                          # absent means hub
client:
  hub_url: https://mesh.example.com   # needed with a join code; a token carries its URL
  join: secret://MESH_JOIN            # secret://NAME or env://NAME, never the code itself
  name: harness-mbp                   # requested; the hub may assign harness-mbp-2
  share: [mesh-probe]                 # local workers the mesh may call; [] shares nothing
```

Store the hub's join code or `iiimj_` token as secret `MESH_JOIN` with
consumers `[mesh]`; the Console's mesh form does this. The value is used once to
enroll. State lives in `$III_COMPOSE_DIR/data/mesh-client` (0700, `join.json`
0600) and later boots reuse its node key. The hub allows 10 join attempts per
address per 10 minutes, so a join the hub refused waits until the join value
changes; an unreachable hub or a missing `hub_url` is retried. `join.json` is
saved as pending before the hub registers the node, so a join cut short after
the hub accepted it reconnects without the spent code; a refused one is deleted.
To disconnect, or to come back after the hub removed the node, call
`mesh::leave` (see [State and removal](#state-and-removal)). Seed the mode from
compose so the first boot is already a client:

```yaml
  mesh:
    worker: path://../../mesh
    start_after: [secrets]
    config_override: { mode: client }   # override only mode; the Console edits client.*
    scripts: { pre_run: make build, run: ./bin/mesh }
```

The client runs on events and polls nothing. It observes
`configuration::on-updated` for its entry, `secrets::on-changed` (a
`secret://` join value) and `engine::on-functions-changed` (the shared workers'
functions) and applies each change at once: `share` and `name` always,
`hub_url` and `join` while it is not joined. A joined client keeps its hub:
another `hub_url` or `join` is used only after `mesh::leave`, and
`mesh::status` reports `join_unused: true` while one is stored. Only `mode`
needs a restart. An `env://NAME` join value is read from the mesh worker's own
environment, which changes only with a restart. A join value or hub URL that
does not parse waits for the save that fixes it. A failure no event announces
the end of (an unreachable hub, configuration or secrets worker, a node the hub
engine disconnected, a failed rename) is retried after 2 seconds and then at
doubling intervals up to a minute, and `mesh::status` reports `retrying: true`
meanwhile.

`mesh::status` and the Console's Mesh page report the hub, assigned name
(`node`), hub namespace, state, each shared worker with its hub function IDs
and, under `remote`, what this machine can call in the mesh, hub first.

The client registers on its local engine a mirror for each function the hub or
another machine shares: `hub::<function>` for the hub's and
`peer::<machine>::<function>` for a machine's, with the owner's description and
schemas. Its own functions are never mirrored back, and a mirror is never
shared again. Calling a mirror calls `mesh::call` on the hub, which runs the
function where it lives. A mirror fails at once with `mesh_offline` while the
client is not connected, and a call in flight ends with `mesh_offline` when the
connection drops. Such a call may still run: the SDK sends a call it queued
once it reconnects, and a call already sent keeps running where it lives. Retry
only what is safe to run twice. The hub re-reads what is shared when the functions on its
engine, its `share` list or a machine's version change. It sends the list in a
signed control call to each machine that holds another revision, and the
mirrors follow. Leaving and removal clear them.

Saving another `client.name` on a joined client renames the machine without
leaving. The client asks the hub over its control connection, which proves the
machine's keys, so a machine renames only itself. The hub applies the rules of
a join: the name is sanitized, and `hub`, the other reserved words, another
machine's name and an engine function prefix get a suffix such as `-2`. The
answer is the name the machine now has, reported as `node`; `requested_name` is
the `client.name` the hub last answered for. Shared function IDs change prefix
(`old::x` becomes `new::x`): the hub closes the machine's sessions under the
old name and the client reconnects by itself, so calls in flight fail and the
shared functions are away for a moment, while the state reads `renaming`. A
rename that fails keeps the old name working, sets `rename_error` until another
name is saved and is asked again with the retry backoff, so a hub that was
unreachable, failed on its side or answered that it cannot rename machines (a
build before renames existed) renames the machine once it can. Configuration
entries named `<old>__*` are not moved. A client joined by an earlier build whose
`client.name` was changed since is renamed when this build first connects.

The hub project and every machine joined to the hub can call shared workers.
Calls reach the local engine as caller `mesh`, and nothing asks here first.
Sharing `ide`, `harness` or any worker that runs commands gives all of them
remote code execution on this machine, so a join code is execute access to
everything shared in the mesh. A machine writes the descriptions and schemas of
its own shared functions, and every other machine lists them under
`peer::<its name>::`. `mesh`, `secrets`, `configuration` and `rbac-proxy`
cannot be shared, and functions under `engine::`, `mesh::`, `mesh-node::`,
`secrets::`, `configuration::`, `rbac-proxy::` and `approval::` are never
published or mirrored. The approval gate keeps agents away only from ids that
start with `approval::`, so a mirrored `peer::<machine>::approval::resolve`
would let an agent on one machine approve another machine's agent calls.
Inbound tailnet connections to the client are refused and the local
`III_TOKEN` is not sent to the hub. The hub gateway is reached through a random
loopback port that accepts a connection only when its request carries a random
per-process token in a header, so other local processes and web pages cannot
act as this node toward the hub. A process that can read this worker's memory
or state directory still can; it already holds the machine's keys.

The hub hands its namespace only to a node that runs its exact version, and
nothing updates itself. On a mismatch the client's status shows the version the
hub wants and a client that starts shares nothing; a client that was already
sharing when the hub restarted on another version keeps its registrations until
it restarts. Only machines on the hub's version take part in sharing: a machine
on another version gets no mirrors, its workers stay out of the other machines'
lists and its calls through the hub are refused. A client that already shares
workers makes them callable by every machine once it runs the hub's version,
without asking again. Build with `make build`, which stamps the `package.json`
version (a plain `go build` has none and fails), and bump clients with the hub.

Bridging is request/response only. Channel refs in a payload or result are
refused with `channels_unsupported`; triggers, observers and events do not
cross machines; a call times out after 10 minutes on the machine that runs it,
and a caller's cancellation does not reach that machine. Only whole workers can
be shared. A mirror keeps its first description and schemas until the function
leaves the list. An owner, the hub or a machine, that shares more than 500
functions or 1 MiB of entries (ids, descriptions and schemas) is left out of
the list: other machines cannot call it, while the hub project still calls a
machine's shares. The hub admits its own shares first, then machines by name,
up to 8 MiB in all. It logs each owner left out, lists the machines under
`catalog_skipped` in its `mesh::status` and marks its own shares `too large`. A
machine may have 64 calls through the hub in flight; the next one fails with
`mesh_busy`. These limits are fixed. Windows clients and payloads over 16 MiB
are unsupported.
Live acceptance against a running hub remains unverified.

## State and removal

The hub keeps its identity and enrollment state in
`$III_COMPOSE_DIR/data/mesh/state.json`. A client keeps its node key, credential
and pinned hub identity in `$III_COMPOSE_DIR/data/mesh-client` (0700,
`join.json` 0600).

Either side can disconnect a client:

- **On the hub**, `mesh::nodes::remove {"name":"<node>"}` or **Remove** on the
  machine's card. The machine stops sharing and loses access to everything
  shared at once, the hub refuses its keys for good, revokes its token and
  replaces the join code. It works while the
  hub has no public URL; the result then carries an empty `code`. The client
  stops and reports `removed by hub` until it leaves.
- **On the client**, `mesh::leave {}` or **Leave hub** on its Mesh page. The
  client logs out, so the hub deletes the machine's record, stops the mesh and
  the bridge, removes its mirrors, and deletes `join.json` and `mesh/`. It
  answers `{"hub_notified":true,"warning":""}`. When the hub cannot be told within 10
  seconds, or the mesh is not running, the local state is deleted all the same
  and `warning` says that the hub keeps listing the machine until it is removed
  there. A client with no enrollment answers `not joined to a hub`. A leave
  waiting behind a join attempt in flight runs before the next attempt.

After a leave the client reports `left hub` and keeps `data/mesh-client/left`,
which holds a SHA-256 digest of the join value the forgotten enrollment was made
with and the leave's warning. `mesh::status` reports that warning as its
`error` until the client joins again, after a restart too. It joins again, as a
new machine, only when `client.join` resolves to another value: the same code
or reusable token is not tried again, after a restart either. A leave cut short
by a restart keeps out all the same, deletes what it had not deleted yet and
warns that the hub may still list the machine. An enrollment saved by an earlier
build does not record its join value; for it, the first value read after the
leave counts as the old one, so leave first and store the new code afterwards.
To rejoin with the same reusable token, delete `data/mesh-client/left`.

A machine the hub removed comes back the same way: **Leave hub**, then a new
join code. A machine removed while it was offline learns it when it next
connects. Its old code and token no longer work, so a new code stored before
the leave is tried at once, whichever build saved its enrollment.

State written before 2026-11-10 may hold machine nodes. They stay listed and can
still connect. On the hub's version the hub hands them an empty worker set, so
they stop the workers they run; on another version it counts them as outdated
and pushes no update. The hub sends no drive calls and drive peers stop seeing
each other, so existing mounts stop answering. To retire such a machine, first
call `mesh::nodes::remove` while its old service still runs: the hub refuses its
keys, revokes its token and replaces the join code, and the service parks. Then
run the old binary's `sudo mesh leave` on the machine to remove its service,
mounts and state. In the reverse order the leave logs the node out, the hub
deletes its record, and `mesh::nodes::remove` fails with `no removable node`
without revoking the token or replacing the code.

Upgrade the hub before its clients, or both together: a hub of the earlier
build shares nothing with a client of this one. To downgrade a hub to a build
before 0.2.0, first remove the `share` key from its entry, even an empty list:
an earlier build refuses the entry and stops at startup.

## Agent access

The [Your machines skill](skills/SKILL.md) describes machine discovery, the
mirrored names and remote paths. Discovery reads are allowed; remote shell/coder
calls require approval by default, even when the local equivalent is allowed.
That includes the mirrored `hub::…` and `peer::…` ids on a joined machine: they
match no allow rule, and deny rules written with `*::`, such as
`!*::shell::pty::*`, cover them too. Approval happens only on the calling
machine; the machine that runs the call does not ask and applies none of its
own rules. A session in `full` mode, an id approved "always" and, in `auto`
mode, an id on the configured always-allow list run without asking and skip the
deny rules too. "Always" approvals are kept by id: a machine that later takes a
removed machine's name inherits those given to its `peer::<name>::…` ids. Node
control (`mesh-node`), proxy, configuration-hook and console internals are
denied to agents.

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
