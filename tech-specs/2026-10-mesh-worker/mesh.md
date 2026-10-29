# mesh

Worker prefix: `mesh::*` (hub), `<node>::mesh-node::*` (node service) ·
Deploy mode: `binary` (Go, release kind `go-binary`, linux/darwin/windows ×
amd64/arm64) · Config id: `mesh`

## Definition

One Go binary with two modes.

- **Hub** (default; `mesh --url ws://127.0.0.1:49134`) is an iii worker on the
  engine's machine. It serves the front door (control, relay, join, install
  scripts), joins its own mesh as the userspace node `hub`, runs the gateway
  on that node, registers `mesh::*`, and answers the `rbac-proxy` hooks of
  the `mesh-gateway` instance.
- **Node** (`mesh join …` installs it; `mesh node` is the service entrypoint)
  is a mesh member with its own TUN interface. It supervises iii workers on
  its machine, and is itself an iii worker through the gateway.

The hub never needs root. A node needs root or admin for its TUN, routes,
hosts file and mounts, and for nothing else (see [Node](#node)).

## Code reuse (decided)

Port Mainplane's network layer; do not write one. Each ported directory keeps
Mainplane's MIT notice, and the coordinator keeps headscale's BSD-3 notice.

| From `mainplane-ai/mainplane` | Lines | Into `mesh/internal/` |
| --- | --- | --- |
| `pkg/mesh` (TUN, per-OS router, hosts block, MSS clamp, IPv6 fragmenter, status socket) | 1220 | `meshnet` |
| `pkg/coordinator` (ts2021 register/map/keepalive/deltas, ephemeral expiry, drive desired state) | 869 | `coordinator` |
| `pkg/auth` (device code, CPace join handler, rate limit) | 424 | `join` |
| `pkg/elevate` | 175 | `elevate` |
| `pkg/relay` | 27 | `relay` |
| `pkg/worker/drives*.go` | 1397 | `drives` |
| service install from `pkg/worker/install_*.go` | — | `service` |

What changes in the port:
- **Names:** `mainplane0` becomes `iii0`, the `.mainplane.net` names become
  `.iii.internal`, the fixed /48 becomes one random ULA /48 per hub, and the
  harness becomes the hub.
- **Removed:** the pointer service, the API-key auth, and Mainplane's own
  worker protocol (run/job/kill/read/write frames). iii workers do that job
  over the bus.

Mainplane's worker protocol and harness are not ported.

`tailscale.com` comes through an iii-owned fork (`go.mod`:
`replace tailscale.com => github.com/iii-hq/tailscale <tag>-iii.N`). The fork
carries the same three patches as `mainplane-ai/tailscale
v1.102.4-mainplane.8`:

1. WebSocket control and DERP clients on every OS, so the front door can sit
   behind Cloudflare or any WebSocket-only proxy. The same patch adds the
   control-key and relay-admission hooks and removes log upload.
2. netcheck plans no STUN probe of a node whose STUN is off.
3. netstack passes an IPv6 first fragment on to reassembly.

Rebase the fork on each Tailscale release we take. Never depend on
Mainplane's fork directly.

## Front door

The hub listens on `listen` (default `0.0.0.0:49180`), and nodes reach it at
`public_url`:

| Route | Purpose |
| --- | --- |
| `GET /key` | the hub's control public key |
| `/ts2021` | Tailscale control: Noise inside an HTTP upgrade or a WebSocket |
| `/derp`, `/derp/probe`, `/derp/latency-check` | the relay; admits registered nodes only |
| `POST /join` | CPace device-code exchange |
| `GET /install.sh`, `GET /install.ps1` | install scripts templated with `public_url` and the hub's version |

Nothing else listens there. Everything on it is either end-to-end encrypted
(Noise, WireGuard inside DERP, CPace) or public (the scripts). A
TLS-terminating proxy in front of it therefore reads no prompt, no command and
no file, unlike exposing the engine.

`public_url` must be stable: a LAN address (`http://10.0.0.5:49180`), the
operator's own domain behind a reverse proxy, or a named Cloudflare tunnel. A
Cloudflare quick tunnel changes URL on restart, and nodes keep the URL they
joined with, so it is unsupported until a rendezvous service exists (a
non-goal). Moving the front door means rejoining every node.

## Configuration

Config lives in the `configuration` worker under id `mesh` (resolved through
`III_CONFIG_NAME` per `docs/sops/configuration.md`). No `config.yaml` is
committed, and `WorkerConfig` defaults seed first boot.

```yaml
public_url: ""                    # required before any join; see Front door
listen: 0.0.0.0:49180             # front door
gateway_port: 49135               # on the hub's mesh node only
proxy_url: ws://127.0.0.1:49201   # the mesh-gateway rbac-proxy instance
stun_port: 0                      # 0 = no STUN; set when the hub has a public UDP port
links: []                         # [[a, b], ...] node pairs that reach each other
nodes: {}                         # <name>: {workers: [ide], allow: []}
drives: {}                        # <drive>: {server: <linux node>, nodes: ["*" | <name>, ...]}
```

- `nodes.<name>.workers` lists the iii workers that node runs. A node with no
  entry runs none, which leaves it on the mesh, reachable and able to use
  drives, with no functions.
- `nodes.<name>.allow` adds function-id globs to that node's default allow
  list (see [Gateway](#gateway)).
- **Hot reload:**
  - `links`, `nodes` and `drives` apply live: through map deltas and
    `mesh-node::*::apply` calls.
  - A change to `listen` or `gateway_port` rebinds the listener, and keeps the
    last good listener and config if the bind fails.
  - A change to `public_url` affects only scripts served afterwards.
- `mesh::on-config-change` is denied to agents. `links` and `drives` change
  only through `configuration::set` or the Console config form, which keep
  their own approval rules.

## State

State that is not config lives in one file, `<data dir>/mesh/state.json`
(mode 0600), owned by the coordinator. It is written by temp file and rename.
It holds:
- the control key, the relay key, and the hub node's key;
- the ULA /48 and the device-code seed;
- the nodes: name, node key, machine key, address, os/arch, operator,
  `joined_at`, `last_seen`, `ephemeral`;
- the refused keys.

Use the repo's worker data-dir convention for `<data dir>`.

A node keeps its own state under `/var/lib/iii-mesh` (Linux),
`/Library/Application Support/iii-mesh` (macOS) or `%ProgramData%\iii-mesh`
(Windows). That state is its keys, its join token, its operator, its worker
binary cache and its logs. Only root or admin may read it.

## Join

- **Device code.** 8 Crockford base32 characters shown as `XXXX-XXXX`, one per
  hub. Case and the hyphen do not matter; O reads as 0, and I and L read as 1.
  `mesh::join::cycle` replaces it. Join secrets derive from the code, so a
  cycle refuses the old secret with no table to clear. Nodes that already
  joined stay.
- **Exchange.** CPace is bound to the hub's control key, so a hub with another
  key derives another shared key. The hub answers with the join secret and its
  control key, sealed. Every exchange counts as a refusal against its source
  address, because the hub cannot tell a right code from a wrong one. Keep
  Mainplane's window and limit.
- **Names.** The hostname is lowercased and reduced to `^[a-z][a-z0-9-]{0,30}$`.
  A name gets `-2`, `-3`… when it would equal one of:
  - another node's name;
  - a top-level prefix of any function registered on the engine;
  - a reserved word: `mesh`, `hub`, `engine`, `iii`, `configuration`,
    `harness`, `shell`, `coder`.

  A node keeps its name while offline, until it is removed.
- **Join token** (phase 3). `mesh::join::token {ephemeral?, expires_in_secs?}`
  returns an opaque `iiimj_…` that carries `public_url`, the hub key and a join
  secret, for unattended installs (`mesh join --token …`). An ephemeral node
  leaves the mesh 3 minutes after its last map poll. It is never part of `"*"`
  in drives and must be named there.

## Node

**Service.** `mesh join` installs and starts the service under the platform's
manager: a systemd unit (Linux), a launchd daemon (macOS) or a Windows service.
It records the operator, the user who ran the command. The service process
holds privilege only for the TUN, routes, hosts file and mounts. Every worker
it starts runs as the operator, never as root.

**Network.**
- **Interface:** `iii0` on Linux, `utunN` on macOS, and on Windows a wintun
  adapter named `iii` with a fixed GUID of its own. `wintun.dll` is downloaded
  from wintun.net and pinned to a sha256.
- **UDP port:** default 41643 (`mesh join --port`).
- **Hosts block:** written between `# iii mesh begin` and `# iii mesh end`. It
  holds `hub` and each peer in the node's map, under a short name and under
  `<name>.iii.internal`. Other lines are left as they are.
- **Coexistence with Tailscale:** the node has its own interface, port and
  state, and changes no DNS settings.
- **Protocol:** addresses are IPv6 only. A program another node should reach
  listens on `::` and is dialled by name.
- **Paths:** direct UDP when the NATs allow it, the hub relay otherwise. The
  mesh moves to a direct path when one opens.

**CLI.**

| Command | Does |
| --- | --- |
| `mesh join <url> <code>` / `--token <t>` | elevates, joins, installs and starts the service |
| `mesh status` | the hub, this node's name and address, and each peer's path (`direct <endpoint>`, `relay hub`, `idle`) |
| `mesh leave` | logs out of the hub, then removes the service, TUN, hosts block, mounts, workers and state |
| `mesh node` | the service entrypoint |

**As an iii worker.** The service dials `ws://hub:49135` with the Go SDK. It
needs no credential (see [Gateway](#gateway)). It registers these functions,
which the gateway prefixes with `<node>::`:

| Function | Input → output |
| --- | --- |
| `mesh-node::status` | `{}` → `{name, address, os, arch, version, peers: [{name, path}], workers: [{name, version, state, restarts, log_tail}], drives: [{name, state, error?}]}` |
| `mesh-node::workers::apply` | `{workers: [{name, version}]}` → per-worker result |
| `mesh-node::drives::apply` | `{serve: [...], mount: [...]}` → per-drive result |

Only the hub calls them. The root `iii-permissions.yaml` denies
`*::mesh-node::*` to agents.

**Supervisor.** For each desired worker, the supervisor does five things:
1. Ensures the binary for this OS and architecture, at the hub's version of
   that worker (or the latest stable when the hub runs none), is in the cache,
   with its sha256 verified.
2. Starts it as the operator with `III_URL=ws://hub:49135`,
   `III_CONFIG_NAME=<node>.<worker>` and `III_WORKER_NAME=<worker>`. Its
   configuration entry is therefore its own, never the hub's.
3. Restarts it with backoff from 1 s up to 60 s.
4. Stops the workers that are no longer desired.
5. Writes each worker's log to the node state dir, rotated.

## Gateway

The gateway is a TCP listener on the hub's userspace node, on `gateway_port`
and reachable from the mesh only. For each connection it:
1. looks up the peer address in its map to get the node name, and refuses an
   unknown peer;
2. mints a ticket: 32 random bytes, single use, 10 s;
3. dials `proxy_url`, copies the upgrade request with any client-sent
   `x-iii-mesh-ticket` removed, and adds its own;
4. pipes bytes both ways until either side closes.

It closes every connection of a node when that node is removed.

It dials a dedicated `rbac-proxy` instance, started with
`III_CONFIG_NAME=mesh-gateway` beside any other proxy the project runs. On
first boot the hub writes this config if the entry is absent, and leaves it
alone after that:

```yaml
host: 127.0.0.1
port: 49201
engine_url: ws://127.0.0.1:49134
rbac:
  auth_function_id: mesh::auth
  on_function_registration_function_id: mesh::on-function-registration
middleware_function_id: mesh::middleware
```

`mesh` registers three functions for the proxy:

- **`mesh::auth`** consumes the ticket and returns
  `{function_registration_prefix: <node>, allowed_functions: default_allow ∪ nodes.<node>.allow, context: {mesh_node: <node>}}`.
  It refuses a missing, used or expired ticket.
- **`mesh::on-function-registration`** refuses a registration whose final id
  is not under `<node>::`, or already belongs to another session.
- **`mesh::middleware`** lets a node session call `configuration::*` only on
  entries named `<node>.*`.

The default allow list is the set of calls the supported workers make at
runtime: configuration, discovery, and whatever else they call. The builder
derives it by running each supported worker behind the gateway (see
[Open items](#open-items)).

## Drives

Phase 2. This section follows Mainplane's behaviour.

- **Config.** A drive is one directory on one server node, mounted by the
  member nodes: `drives: {proj: {server: linuxbox, nodes: ["*"]}}`. `"*"` means
  every persistent node.
- **Server.**
  - The server must be a Linux node with apt. Its first drive installs
    `nfs-kernel-server` and `samba`, and then runs a dedicated smbd
    (`iii-mesh-smbd`) that listens on the mesh address only.
  - Exports go to member addresses only, from `/srv/iii-mesh/<drive>`. Every
    file on a drive belongs to the server's operator.
  - A machine that already exports anything over NFS, or has a program on
    port 445, is refused. The refusal shows in `mesh-node::status` and in the
    hub log.
- **Mounts.**
  - Linux: `/drives/<drive>`, NFSv4.2.
  - macOS: `/Volumes/<drive>`, NFSv4.0.
  - Windows: a letter from `W:` down, SMB3, mounted in the operator's session.
- **Behaviour.**
  - While a server is unreachable, file operations wait.
  - Mounts survive restarts and updates.
  - A node removed from a drive or from the mesh unmounts it.
  - A server left with no drives removes its exports and smbd, and keeps the
    packages and `/srv/iii-mesh`.
- **Locking.** Lock with fcntl (`lockf`), not `flock`: on the server, `flock`
  does not see other nodes' locks. The skill says so.

## Functions

Hub functions:

| Function | Input → output | Permission |
| --- | --- | --- |
| `mesh::status` | `{}` → `{version, public_url, prefix, hub_address, front_door, gateway, proxy, nodes: {total, online}}`; each check is `ok` or the error | allow |
| `mesh::nodes::list` | `{}` → `[{name, address, os, arch, online, last_seen, ephemeral, path, workers: [{name, state}], drives: [{name, state}]}]` | allow |
| `mesh::nodes::get` | `{name}` → one entry above plus that node's `mesh-node::status` | allow |
| `mesh::nodes::remove` | `{name}` → `{removed: true}` | needs approval |
| `mesh::join::code` | `{}` → `{code, install: {unix, windows}}` | needs approval (it is a credential) |
| `mesh::join::cycle` | `{}` → `{code, install}` | needs approval |
| `mesh::join::token` (phase 3) | `{ephemeral?, expires_in_secs?}` → `{token, install}` | needs approval |

Internal functions, denied to agents: `mesh::auth`, `mesh::middleware`,
`mesh::on-function-registration`, `mesh::on-config-change`,
`mesh::ui-content`, `mesh::configuration-id`.

Trigger type `mesh::node-changed` (phase 3) delivers
`{name, event: joined|online|offline|removed, at}`. Its binding config is
`{node?}`. Delivery is best effort and nothing is replayed, matching the
devin and hermes feeds.

## Agent exposure

**Skill.** `skills/SKILL.md` ("Your machines") tells the agent five things:
- `mesh::nodes::list` shows the machines.
- A machine's functions are the usual ids under its name: `laptop::shell::exec`
  and `laptop::coder::read-file` take the same input as `shell::exec` and
  `coder::read-file`.
- An offline machine has no functions.
- Where drives mount on each OS, and to lock with fcntl.
- Never call `*::mesh-node::*`.

**Permissions** (root `iii-permissions.yaml`):
- Allowed: `mesh::status`, `mesh::nodes::list`, `mesh::nodes::get`, plus
  `*::coder::{info,read-file,search,list-folder,tree}`, prefixed mirrors of the
  read-only `coder::*` rules.
- Denied: `*::mesh-node::*` and the internal functions.
- Everything else is unlisted, so it needs approval. That includes every
  remote `shell::*` and every remote write.

**Required harness change (phase 1).** `harness/src/filesystem_scope.rs`
`is_scoped_function` matches only ids that start with `shell::` or `coder::`.
For `laptop::coder::read-file` the harness therefore neither stamps the
session's `fs_scope` nor strips a scope the model supplied, and the remote
`ide` would trust a model-chosen root. The change makes stamping and
stripping also apply when the id after one leading `<segment>::` is a scoped
id. It ships before or with phase 1, with a test for a prefixed id carrying a
model-supplied `fs_scope`.

## Boundaries

- The engine and the `mesh-gateway` proxy listen on loopback only. The gateway
  listens on the mesh only. The front door carries no bus traffic.
- A node registers only under `<node>::`, calls only its allow list, and holds
  no engine credential.
- Removed keys are refused forever. A cycled code refuses its old join secret.
- Nodes make outbound connections only to:
  - `public_url`;
  - linked peers and the hub, over UDP;
  - the release CDN, for the `mesh` binary and worker binaries;
  - wintun.net, from the installer on Windows.

  The binary makes no other connection: no log upload, no Tailscale DERP, no
  STUN unless `stun_port` is set.
- Workers on nodes run as the operator.
- `mesh` changes no DNS settings and no hosts-file line outside its block.

## Dependencies

- **`configuration`** (required): the `mesh` config, the `mesh-gateway`
  proxy config, and each node worker's `<node>.<worker>` entry.
- **`rbac-proxy`** (required): a second instance named `mesh-gateway`.
- **`ade`** (optional, phase 2): the Console page, which shows nodes, path,
  workers and drives, and the join code with its one-liners. Links, drives
  and node workers are edited through the generated config form.
- **Go:**
  - `iii` Go SDK;
  - the `tailscale.com` fork;
  - `filippo.io/cpace`.
- **On nodes:** apt (drive servers) and wintun (Windows).

## File layout

Follow `whatsapp/` (the repo's Go worker) and `docs/sops/go-binary-worker.md`:

- **Top level:** `iii.worker.yaml` (`language: go`, `deploy: binary`,
  `manifest: package.json`), `package.json` (version), `Makefile`
  (`CGO_ENABLED=0 go build -trimpath`), `go.mod`, `main.go`, `NOTICE`.
- **`internal/`:** `{coordinator,meshnet,join,elevate,relay,drives,service,gateway,supervisor,hub}/`.
- **`skills/`:** `SKILL.md`.
- **Also:** `iii-permissions.yaml`, `README.md`, and `ui/` (phase 2).
- **Repo files to touch:**
  - a `.deploy/workers.yaml` entry with `kind: go-binary` and Windows targets;
  - the root `iii-permissions.yaml` rules;
  - the harness change above.

## Testing

**Unit tests:**
- links → who sees whom;
- device-code parsing (case, hyphen, O/0, I/L/1);
- tickets: single use and expiry;
- name sanitizing and collisions;
- a drive's desired state per node;
- the harness change for prefixed scoped ids.

**Integration** (Linux CI, no root):
- A hub and two userspace nodes (tsnet) in one test process, with a real
  engine and `rbac-proxy`.
- Join, then a map without links (the nodes cannot see each other), then with
  a link (they can).
- Removal reaches the peers in under 5 s.
- A fake worker on a node registers `echo`, which appears as `<node>::echo`
  and answers.
- Registering an id that already exists, or one outside the prefix, is
  refused.
- A loopback client that dials the proxy without a ticket is refused.

**Machine matrix** (acceptance): a fresh Ubuntu 26.04 VM, a Mac on another
network, Windows 11 (phase 2), and the hub behind a named Cloudflare tunnel.

## Acceptance criteria

1. The one-liner joins a fresh Ubuntu VM and a Mac on another network in under
   a minute each. `mesh::nodes::list` shows both online, and neither machine
   opened an inbound port.
2. `iii trigger mac::shell::exec '{"command":"uname","args":["-a"]}'` returns
   the Mac's kernel. A harness agent asked "what OS does each of my machines
   run?" answers through the skill, with no commands spelled out for it.
3. Two unlinked nodes cannot reach each other. After `links: [[a, b]]` they
   can, by short name, within 5 s.
4. Within 10 s of `mesh::nodes::remove mac`:
   - its functions are gone from `engine::functions::list`;
   - its peers have lost its name;
   - its TUN, hosts block and workers are gone.

   Joining again needs `mesh leave` and a new join.
5. Brute-force and cycle limits hold:
   - After Mainplane's limit of wrong codes from one address, that address is
     refused for the window, and the right code works afterwards.
   - After `mesh::join::cycle` the old code is refused, and joined nodes are
     unaffected.
6. A node's Tailscale keeps working before, during and after join and leave:
   `tailscale status` and MagicDNS names both work.
7. With the hub behind a named Cloudflare tunnel, joins and relayed paths
   work. `mesh status` shows `relay hub` when UDP is blocked and `direct` on
   a LAN.
8. On a drive served by the Ubuntu node:
   - a file written on the Mac at `/Volumes/proj` appears on Windows `W:`
     and on Linux `/drives/proj`;
   - an fcntl lock held on one node is seen on another.
9. Node registrations are confined:
   - A node registering `mesh::nodes::list` or an id outside its prefix is
     refused.
   - A `laptop::coder::read-file` call with a model-supplied `fs_scope` runs
     under the session's stamped scope, not the model's.
10. A capture at the front door shows no plaintext except the install scripts
    and CPace messages. No node opens a connection to a Tailscale-run host.
11. Phase 3:
    - An ephemeral node joined by token, idle for 3 minutes, is gone from the
      mesh.
    - A function bound to `mesh::node-changed` sees `online` and `offline`
      when a node's service stops and starts.
    - `browser` runs on a node as `<node>::browser::*`.

## Open items

Verify these first; each one changes code outside `mesh` or the default allow
list.

1. **Trigger types through the prefix.** Does `rbac-proxy` prefix trigger
   types and trigger registrations from a prefixed session, such as `ide`'s
   `shell::job-finished`? If it does not, add that to `rbac-proxy` (the shared
   layer), not to `mesh`.
2. **Can the hooks refuse?** Confirm that
   `on_function_registration_function_id` can refuse a registration, and that
   `middleware_function_id` sees the calls a proxied session makes, with their
   payload. If either falls short, add what is missing to `rbac-proxy`.
3. **Worker binaries on nodes.** Reuse the iii CLI's registry resolution if it
   can install a worker binary outside an engine project. Otherwise fetch the
   same release artifacts the CLI fetches, verified by sha256. Do not write a
   second registry client if one is reusable.
4. **Default allow list.** Run `ide` (then `browser`, `computer`, `tmux`)
   behind the gateway and record every call each makes. That list is
   `default_allow`.
5. **Console forms for node workers.** Check whether the Console finds them
   through `<node>::ide::configuration-id`. If it does not, that is a
   follow-up; config stays editable through `configuration::set` on
   `<node>.<worker>`.
6. **`ide` on Windows.** It ships no Windows build today, so a Windows node
   has no `shell::*` or `coder::*` until it does. Phase 2 ships Windows nodes
   either way, and the skill says which machines lack them.
7. **Console page from Go.** The Console page helper (`iii_console_ui`) is
   Rust. Register `mesh::ui-content` at the protocol level from Go, per
   `docs/sops/injectable-console-ui.md`.
