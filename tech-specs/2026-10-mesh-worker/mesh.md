# mesh

Worker prefix: `mesh::*` (hub), `<node>::mesh-node::*` (node service) ·
Deploy mode: `binary` (Go, release kind `go-binary`, linux/darwin/windows ×
amd64/arm64) · Config id: `mesh`

## Definition

One Go binary with two modes.

- **Hub** (default; `mesh --url ws://127.0.0.1:49134`) is an iii worker on the
  engine's machine. It serves the front door (control, relay, join), joins its
  own mesh as the userspace node `hub`, runs the gateway on that node,
  registers `mesh::*`, and owns the project's `rbac-proxy` configuration.
- **Node** (`mesh join …` installs it; `mesh node` is the service entrypoint)
  is a mesh member with its own TUN interface. It supervises iii workers on
  its machine, and is itself an iii worker through the gateway.

The hub never needs root. A node needs root or admin for its TUN, routes,
hosts file, mounts and self-update, and for nothing else (see [Node](#node)).

## Code reuse (decided)

Port Mainplane's network layer; do not write one. Each ported directory keeps
Mainplane's MIT notice, and the coordinator keeps headscale's BSD-3 notice.

| From `mainplane-ai/mainplane` | Lines | Into `mesh/internal/` |
| --- | --- | --- |
| `pkg/mesh` (TUN, per-OS router, hosts block, MSS clamp, IPv6 fragmenter, status socket) | 1220 | `meshnet` |
| `pkg/coordinator` (ts2021 register/map/keepalive/deltas, ephemeral expiry, signed `/id`, drive desired state) | 869 | `coordinator` |
| `pkg/auth` (device code, CPace join handler, rate limit), API-key half dropped | 424 | `join` |
| `pkg/elevate` | 175 | `elevate` |
| `pkg/relay` | 27 | `relay` |
| `pkg/worker/drives*.go` | 1397 | `drives` |
| `pkg/worker/install_*.go` (systemd, launchd, Windows service) | 779 | `service` |
| `pkg/worker/update.go` (download and replace self) | — | `service` |

What changes in the port:
- **Names:** `mainplane0` becomes `iii0`, `.mainplane.net` names become
  `.iii.internal`, the fixed /48 becomes one random ULA /48 per hub, and the
  harness becomes the hub.
- **Removed:** the pointer service, API keys, and Mainplane's own worker
  protocol (`run` frames, file frames). iii workers do that job over the bus.
- **Updates:** a node updates itself when the hub's `mesh-node::update` call
  tells it to, instead of on a worker-protocol frame.
- **Rate limit:** see [Front door](#front-door).

Mainplane's harness and its HTTP API are not ported.

`tailscale.com` comes through an iii-owned fork (`go.mod`:
`replace tailscale.com => github.com/iii-hq/tailscale <tag>-iii.N`). The fork
carries the same three patches as `mainplane-ai/tailscale
v1.102.4-mainplane.8`:

1. **WebSocket control and DERP clients on every OS**, so the front door can
   sit behind Cloudflare or any WebSocket-only proxy. WebSocket control is
   opt-in through `TS_CONTROL_WEBSOCKET`, which the node service sets when
   `public_url` needs it. The same patch adds the control-key and
   relay-admission hooks, and removes log upload, the sockstat logger, and
   the DNS fallback to Tailscale's DERP servers.
2. **netcheck** plans no STUN probe of a node whose STUN is off.
3. **netstack** passes an IPv6 first fragment on to reassembly.

Rebase the fork on each Tailscale release we take. Never depend on
Mainplane's fork directly.

## Front door

The hub listens on `listen` (default `0.0.0.0:49180`), and nodes reach it at
`public_url`:

| Route | Purpose |
| --- | --- |
| `GET /id` | the hub's identity key, and its control key signed by it |
| `/ts2021` | Tailscale control: Noise inside an HTTP upgrade or a WebSocket |
| `/derp`, `/derp/probe`, `/derp/latency-check` | the relay; admits registered nodes only |
| `POST /join` | CPace device-code exchange |

Nothing else listens there, and the hub serves nothing executable. Install
scripts and binaries come from the `mesh` GitHub release over HTTPS (see
[Join](#join)).

Everything on the front door is either end-to-end encrypted (Noise, WireGuard
inside DERP), self-authenticating (CPace, the signed `/id`), or harmless. So
`public_url` may be plain HTTP on a LAN, and a TLS-terminating proxy in front
of it reads no prompt, command or file.

**Stable URL.** `public_url` must be one of:
- a LAN address (`http://10.0.0.5:49180`);
- the operator's own domain behind a reverse proxy;
- a named Cloudflare tunnel.

A Cloudflare quick tunnel changes URL on restart, and nodes keep the URL they
joined with, so it is unsupported until a rendezvous service exists (a
non-goal). Moving the front door means rejoining every node.

**Rate limit.** Each `/join` exchange counts as one refusal against the
client address. An address that reaches 10 refusals in 10 minutes is refused
until that window ends. There is no global limit, so one attacker cannot block
every join; the code's 40 bits carry the rest.

The client address is the TCP peer. The hub reads `CF-Connecting-IP` or the
first `X-Forwarded-For` entry only when the peer is listed in
`trusted_proxies`. A direct LAN client and a client behind the proxy therefore
both work, and a client cannot set its own address.

## Configuration

Config lives in the `configuration` worker under id `mesh` (resolved through
`III_CONFIG_NAME` per `docs/sops/configuration.md`). No `config.yaml` is
committed, and `WorkerConfig` defaults seed first boot.

```yaml
public_url: ""                    # required before any join; see Front door
listen: 0.0.0.0:49180             # front door
trusted_proxies: []               # CIDRs whose forwarded-for headers count, e.g. [127.0.0.1/32, ::1/128] behind cloudflared
gateway_port: 49135               # on the hub's mesh node only
proxy_url: ws://127.0.0.1:49200   # the project's rbac-proxy, which mesh owns
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
  - `links`, `nodes`, `drives` and `trusted_proxies` apply live: through map
    deltas and `mesh-node::*::apply` calls. A change to a node's grants also
    closes and re-authenticates its sessions (see [Gateway](#gateway)).
  - A change to `listen` or `gateway_port` rebinds the listener, and keeps the
    last good listener and config if the bind fails.
  - `public_url` is baked into the one-liners at the moment
    `mesh::join::code` returns them.
- `mesh::on-config-change` is denied to agents. `links` and `drives` change
  only through `configuration::set` or the Console config form, which keep
  their own approval rules.

## State

State that is not config lives in one file, `<data dir>/mesh/state.json`
(mode 0600), owned by the coordinator. It is written by temp file and rename.
It holds:
- the identity key (ed25519), the control key, the relay key, and the hub
  node's key;
- the ULA /48 and the device-code seed;
- the nodes: name, node key, machine key, address, os/arch, version, operator,
  `joined_at`, `last_seen`, `ephemeral`;
- the refused keys;
- in phase 3, the issued join tokens: a hash of each token's secret, its
  expiry, whether it is reusable and ephemeral, and the nodes it created.

Use the repo's worker data-dir convention for `<data dir>`.

A node keeps its own state under `/var/lib/iii-mesh` (Linux),
`/Library/Application Support/iii-mesh` (macOS) or `%ProgramData%\iii-mesh`
(Windows). That state is its keys, the pinned hub identity key, its operator,
its worker binary cache and its logs. Only root or admin may read it.

## Join

**Device code.**
- 8 Crockford base32 characters shown as `XXXX-XXXX`, one per hub. Case and
  the hyphen do not matter; O reads as 0, and I and L read as 1.
- `mesh::join::cycle` replaces it, and so does every `mesh::nodes::remove`.
  Join secrets derive from the code, so a cycle refuses every old secret with
  no table to clear. Nodes that already joined stay.

**Exchange.** Port Mainplane's scheme:
1. CPace binds to the hub's identity key, so a hub with another key derives
   another shared key.
2. The hub seals the join secret under the shared key.
3. The node pins the identity key, and trusts a control key only as `/id`
   signs it.

A man in the middle gets one guess at the code and nothing else.

**One-liners** (from `mesh::join::code`):

```sh
curl -fsSL https://github.com/iii-hq/workers/releases/download/<mesh release>/install.sh | sh -s -- <public_url> XXXX-XXXX
```

```powershell
& ([scriptblock]::Create((irm https://github.com/iii-hq/workers/releases/download/<mesh release>/install.ps1))) <public_url> XXXX-XXXX
```

`<mesh release>` is the hub's own release. The builder uses the asset URL
scheme that `go-binary` releases actually publish. The script:
1. downloads `mesh` for the machine's OS and architecture from the same
   release;
2. checks it against the release's sha256 list;
3. runs `mesh join` elevated.

**Names.** The hostname is lowercased and reduced to `^[a-z][a-z0-9-]{0,30}$`.
A name gets `-2`, `-3`… when it would equal one of:
- another node's name;
- a top-level prefix of any function registered on the engine;
- a reserved word: `mesh`, `hub`, `engine`, `iii`, `configuration`,
  `harness`, `shell`, `coder`.

A node keeps its name while offline, until it is removed.

**Join token** (phase 3). `mesh::join::token {ephemeral?, reusable?, expires_in_secs?}`
returns `iiimj_…`, which carries `public_url`, the hub identity key and a
random secret that state holds only as a hash. It is for unattended installs
(`mesh join --token …`). Because each token has its own secret, it can
expire, and `mesh::join::revoke {token_id}` revokes it, all without touching
the device code.

A token must not undo a removal:
- **Single use by default.** Its first registration consumes it.
- **Reusable only for CI.** `reusable: true` is accepted only with
  `ephemeral: true`, for CI runners that join again and again.
- **Removal revokes it.** State records which token created each node, and
  `mesh::nodes::remove` revokes that token. A removed machine that kept its
  token therefore cannot register fresh keys with it.

An ephemeral node leaves the mesh 3 minutes after its last map poll. It is
never part of `"*"` in drives and must be named there.

## Node

**Service.** `mesh join` installs and starts the service under the platform's
manager: a systemd unit (Linux), a launchd daemon (macOS) or a Windows service.
It records the operator, the user who ran the command. The service process
holds privilege only for the TUN, routes, hosts file, mounts and self-update.
Every worker it starts runs as the operator, never as root.

**Network.**
- **Interface:** `iii0` on Linux, `utunN` on macOS, and on Windows a wintun
  adapter named `iii` with a fixed GUID of its own. `wintun.dll` is downloaded
  from wintun.net over HTTPS and pinned to a sha256.
- **UDP port:** default 41643 (`mesh join --port`).
- **Hosts block:** written between `# iii mesh begin` and `# iii mesh end`. It
  holds `hub.iii.internal` and `<peer>.iii.internal` for each peer in the
  node's map. It writes no short names, so it never shadows a MagicDNS or LAN
  name. Other lines are left as they are.
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

**As an iii worker.** The service dials `ws://hub.iii.internal:49135` with the
current Go SDK. It needs no credential (see [Gateway](#gateway)). It registers
these functions, which the gateway prefixes with `<node>::`:

| Function | Input → output |
| --- | --- |
| `mesh-node::status` | `{}` → `{name, address, os, arch, version, peers: [{name, path}], workers: [{name, version, state, restarts, log_tail}], drives: [{name, state, error?}]}` |
| `mesh-node::update` | `{version}` → downloads `mesh` for this target from that GitHub release, checks its sha256, replaces itself and restarts the service |
| `mesh-node::workers::apply` | `{workers: [{name, artifact_url, sha256, version}]}` → per-worker result |
| `mesh-node::drives::apply` | `{serve: [...], mount: [...]}` → per-drive result |

**Only the hub can drive them.** Every `mesh-node::*` request is
`{payload, ts, nonce, sig}`, where `sig` is the hub identity key's ed25519
signature over the function id, payload, `ts` and `nonce`. The node pinned
that key at join. It refuses:
- an unsigned request;
- a bad signature;
- a `ts` more than 60 s from its clock;
- a nonce it has seen in that window.

So no agent can drive a node's supervisor or drives, however the call
reaches it: directly, through `fp::pipe` or `fp::batch`, or through any other
worker. The root `iii-permissions.yaml` also denies `*::mesh-node::*` to
agents, and `fp` refuses those ids (see
[Required changes](#required-changes-outside-mesh)).

**Versions.** When a node connects, the hub compares versions. If they
differ, it calls `mesh-node::update` before it applies workers or drives. The
`mesh-node::*` shapes stay compatible within a major version. A node on
another major gets only `update`, and shows as `outdated` until it updates.

**Supervisor.** The hub resolves every worker in `nodes.<name>.workers` to an
artifact for that node's target: an HTTPS URL and a sha256 (see
[Required changes](#required-changes-outside-mesh)). For each, the
supervisor:
1. downloads it into the cache and checks the sha256;
2. starts it as the operator with this environment:
   - `III_URL=ws://hub.iii.internal:49135`;
   - `III_NAMESPACE=<the hub's namespace>`, so the harness finds its
     functions;
   - `III_WORKER_NAME=<worker>`. The client cannot be trusted to choose a
     unique name: the engine keeps one live worker per name, and a malicious
     node could take `ide` and lock another node's `ide` out. So `rbac-proxy`
     names a prefixed session's worker `rbac-proxy:<prefix>::<name>`, from
     the authenticated grant (see
     [Required changes](#required-changes-outside-mesh)).
   - `III_CONFIG_NAME=<node>__<worker>`, so its configuration entry is its
     own. The engine accepts only `[a-z0-9_-]{1,64}` as a configuration id
     (`ConfigurationStore::validate_id`). Node names contain no `_`, so the
     `__` separator cannot be ambiguous: `lap__x` and `laptop__x` never
     overlap.
3. restarts it with backoff from 1 s up to 60 s;
4. stops the workers that are no longer desired;
5. writes each worker's log to the node state dir, rotated.

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

**The proxy.** `mesh` owns the project's single `rbac-proxy` (config id
`rbac-proxy`). Running a second instance does not work today: three of its
function ids are fixed, and the engine refuses them from a second worker. On
boot the hub reads the entry and acts on what it finds:

- **No entry, or no `rbac.auth_function_id`** (a proxy that seeded its
  defaults is closed): the hub writes the block below.
- **The `mesh` block** (`auth_function_id: mesh::auth`): the hub leaves it.
- **Another auth function:** the hub writes nothing, leaves the gateway down,
  and reports `proxy: owned by <function>` in `mesh::status`.

```yaml
host: 127.0.0.1
port: 49200
engine_url: ws://127.0.0.1:49134
middleware_function_id: mesh::middleware
rbac:
  auth_function_id: mesh::auth
  on_trigger_registration_function_id: mesh::on-trigger-registration
```

`mesh` registers three functions for the proxy. The proxy resolves the
middleware in the caller's namespace (`rbac-proxy/src/interceptor.rs`), and
node workers call `configuration::*` in `default`, as the configuration
contract requires (`ide/src/configuration.rs`). So the hub registers
`mesh::middleware` and `mesh::on-trigger-registration` both in its own
namespace and in `default`. When the two differ, that takes a second SDK
connection pinned to `default`; both connections serve the same session-key
table.

- **`mesh::auth`** consumes the ticket. It refuses a missing, used or expired
  one. Otherwise it returns:
  - `function_registration_prefix: <node>`;
  - namespace-scoped grants (only these take globs; top-level
    `allowed_functions` is exact match):
    - `<hub namespace>: default_allow ∪ nodes.<node>.allow`;
    - `default: [configuration::get, configuration::ensure, configuration::register, configuration::set]`,
      which the middleware then confines.

    When the hub runs in `default`, the two merge.
  - `allow_trigger_type_registration: true` and
    `allowed_trigger_types: [<node>::*, configuration, …default_trigger_types]`;
  - `context: {mesh_node: <node>, session_key: <32 random bytes>}`. The hub
    keeps the session key live until the gateway connection closes.
- **`mesh::middleware`** receives every call a node makes except `engine::*`,
  with its payload and context, *instead of* the target.
  - **Session check.** It first requires a live `context.session_key`. An
    agent that calls `mesh::middleware` itself, directly or through `fp`, has
    no key and is refused. Without this check the middleware would forward
    the agent's calls with `mesh`'s own authority.
  - **Configuration.** It confines `configuration::*` to entries named
    `<node>__*`, and never lets hub environment values reach a node:
    - `configuration::get` is forwarded with `raw: true`;
    - `ensure`, `register` and `set` are forwarded with `expand: false` (see
      [Required changes](#required-changes-outside-mesh));
    - every other `configuration::*` call is refused.

    Without this, a node could seed `<node>__probe` with `${OPENAI_API_KEY}`
    and read the key back expanded.
  - **Everything else** is forwarded to its target, and the target's result
    is returned. The timeout follows the proxy's 610 s middleware budget.
- **`mesh::on-trigger-registration`** also requires a live session key in the
  context. It refuses:
  - any `harness::hook::*` binding: a hook sees, and can rewrite, the
    arguments and results of every call on the hub, approved ones included;
  - a `configuration` binding whose filter names an entry outside `<node>__*`;
  - a binding to any type outside the grant;
  - a binding whose config has any key outside that type's known shape.

    This covers `condition_function_id` in particular. The engine runs a
    trigger's condition function with its own authority, outside the proxy,
    so a node naming `tailscale::logout` as the condition of its own binding
    would run a function it was never granted. The same rule is also
    enforced in `rbac-proxy` itself (see
    [Required changes](#required-changes-outside-mesh)).

  `ide` binds `harness::hook::pre-turn`, `post-turn`, `pre-trigger` and
  `post-trigger` for its turn log (`ide/src/turns.rs`). A refused binding
  only logs a warning and leaves a remote `ide`'s turn log without session
  history. That is the intended v1 behaviour: remote hooks are a non-goal.

Functions are confined without a hook: the proxy prefixes every registration,
and the engine already refuses a duplicate id.

**A trigger type may be granted to nodes only when its deliveries can be
confined to the node's own data.** `default_allow` and
`default_trigger_types` are the calls and bindings the supported workers make
at runtime that pass that test. The builder derives them by running each
supported worker behind the gateway (see [Open items](#open-items)). In v1
they never include:
- `harness::hook::*`;
- state or stream subscriptions;
- `queue`, `http` or `cron` types;
- any type whose deliveries carry other sessions' data.

**Grant changes revoke live sessions.** Grants are fixed when a connection
authenticates. So when anything that feeds a node's grants changes, the
gateway closes every connection that node has, and drops their session keys.
That covers `nodes.<name>.allow`, `nodes.<name>.workers`, the default lists
after a hub upgrade, and removal. The workers reconnect and authenticate
against the current grants. The engine drops the old sessions' registrations
and trigger bindings on disconnect, so nothing granted earlier survives.

## Drives

Phase 2. This section follows Mainplane's behaviour.

- **Config.** A drive is one directory on one server node, mounted by the
  member nodes: `drives: {proj: {server: linuxbox, nodes: ["*"]}}`. `"*"` means
  every persistent node.
- **Server.**
  - The server must be a Linux node with apt. Its first drive installs
    `nfs-kernel-server` and `samba`.
  - It runs a dedicated smbd unit (`iii-mesh-smbd`) whose systemd
    `IPAddressDeny=any` / `IPAddressAllow=<mesh /48>` admits only mesh
    traffic.
  - Exports go to member addresses only, from `/srv/iii-mesh/<drive>`. Every
    file on a drive belongs to the server's operator.
  - A machine that already exports anything over NFS, or has a program on
    port 445, is refused. The refusal shows in `mesh-node::status` and in the
    hub log.
- **Mounts.**
  - Linux: `/drives/<drive>`, NFSv4.
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
| `mesh::status` | `{}` → `{version, public_url, prefix, hub_address, front_door, gateway, proxy, nodes: {total, online, outdated}}`; each check is `ok` or the error | allow |
| `mesh::nodes::list` | `{}` → `[{name, address, os, arch, version, online, last_seen, ephemeral, path, workers: [{name, state}], drives: [{name, state}]}]` | allow |
| `mesh::nodes::get` | `{name}` → one entry above plus that node's `mesh-node::status` | allow |
| `mesh::nodes::remove` | `{name}` → `{removed: true, code, install}`; the code has been cycled | needs approval |
| `mesh::join::code` | `{}` → `{code, install: {unix, windows}}` | needs approval (it is a credential) |
| `mesh::join::cycle` | `{}` → `{code, install}` | needs approval |
| `mesh::join::token` (phase 3) | `{ephemeral?, reusable?, expires_in_secs?}` → `{token_id, token, install}`; `reusable` requires `ephemeral` | needs approval |
| `mesh::join::revoke` (phase 3) | `{token_id}` → `{revoked: true}` | needs approval |

Internal functions, denied to agents: `mesh::auth`, `mesh::middleware`,
`mesh::on-trigger-registration`, `mesh::on-config-change`,
`mesh::ui-content`, `mesh::configuration-id`.

Trigger type `mesh::node-changed` (phase 3) delivers
`{name, event: joined|online|offline|removed|outdated, at}`. Its binding
config is `{node?}`. Delivery is best effort and nothing is replayed,
matching the devin and hermes feeds.

## Agent exposure

**Skill.** `skills/SKILL.md` ("Your machines") tells the agent six things:
- `mesh::nodes::list` shows the machines.
- A machine's functions are the usual ids under its name: `laptop::shell::exec`
  and `laptop::coder::read-file` take the same input as `shell::exec` and
  `coder::read-file`.
- Paths are the machine's own, under that machine's configured roots.
- An offline machine has no functions, and a Windows machine has no `shell::*`
  or `coder::*` until `ide` ships for Windows.
- Where drives mount on each OS, and to lock with fcntl.
- Never call `*::mesh-node::*`.

**Permissions** (root `iii-permissions.yaml`):
- Allowed: `mesh::status`, `mesh::nodes::list`, `mesh::nodes::get`.
- Denied: `*::mesh-node::*`, the internal functions, and prefixed mirrors of
  `ide`'s hard denies (`*::shell::pty::*`, `*::shell::scm::*`,
  `*::shell::ui-state::*`, `*::shell::config-status`,
  `*::shell::kill-session-jobs`, `*::shell::on-config-change`). A glob's `*`
  also matches `::`, so a mirrored deny can only deny more than it names.
  That is safe.
- No allow is mirrored: an allow glob like `*::coder::read-file` would also
  allow any other proxy tenant's ids. Every remote call therefore needs
  approval in v1. A per-machine allow list is a follow-up that needs
  prefix-aware rule matching in the permission engine.

## Required changes outside mesh

These ship before or with phase 1.

1. **Harness and fp: remote calls get no hub scope.**
   - **Where:** `harness/src/filesystem_scope.rs` `is_scoped_function` and
     `fp/src/pipe.rs` `is_scoped_step` both match only ids starting with
     `shell::` or `coder::`.
   - **Problem:** for `laptop::coder::read-file`, the model's own `fs_scope`
     goes through untouched, and `ide` adds its root and grants to the
     allowed roots (`ide/src/code/path.rs`).
   - **Change:** for an id whose remainder after one leading `<segment>::` is
     a scoped id, strip any `fs_scope` and stamp none. Stamping the hub's root
     would not help: `ide` canonicalizes it on the node, where the path does
     not exist.
   - **Shape:** one shared predicate used by both, plus a test per call path:
     a direct call, `fp::pipe` and `fp::batch`.
   - **fp refused steps.** `fp`'s refused-step ids are exact today
     (`fp/src/pipe.rs`). The same predicate makes them also refuse:
     - `<segment>::<refused id>`;
     - `*::mesh-node::*`;
     - `mesh`'s internal functions.

     A pipe or batch can then no longer reach what a direct call is denied.

   The node's `ide` then applies its own configured roots from `<node>__ide`.
   Approval-gate's access-watch binds only unprefixed `shell::*` / `coder::*`
   and does not cover remote calls, which the permission rules above route to
   approval instead.
2. **rbac-proxy: a prefix confines everything the session names.** In
   `rbac-proxy/src/interceptor.rs`, for a session with
   `function_registration_prefix`:
   - **Trigger types.** Ids are registered bare today, and engine→worker
     `registertrigger` frames are never un-prefixed. A node's `ide` would
     either be refused its `shell::job-finished` type or clash with the hub's.
     The change prefixes trigger type ids from the session, and strips the
     prefix on frames back to it.
   - **Worker name.** Today it is `rbac-proxy:<name>`, with the name the client
     chose. The change makes it `rbac-proxy:<prefix>::<name>`, so a session
     can never hold a name that belongs to another prefix.
   - **Condition functions.** A trigger registration's
     `condition_function_id` (and any other function id in its config) must
     name a function the session registered, or one it may call. It is
     refused otherwise, because the engine runs it with its own authority.
3. **A resolver for worker artifacts.** The hub needs, for a worker reference
   and a target (os/arch), an HTTPS artifact URL and its sha256.
   `iii-compose`'s `registry::install` does this without a project, but it is
   Rust under the Elastic License 2.0, so the Go hub cannot link it. If
   compose exposes resolution as a bus function, use it. Otherwise add one
   there, so registry logic keeps one owner. Do not write a registry client
   in `mesh`.
4. **configuration: entries that never expand.** Today `configuration::get`
   expands `${VAR:default}` against the configuration process's environment
   unless the read is `raw: true` (`docs/sops/configuration.md`).
   - **The change:** `ensure`, `register` and `set` accept `expand: false`,
     stored with the entry. For such an entry, `get`, `list`, change-trigger
     payloads and mutator results always return the stored value unexpanded.
   - **Who sets it:** `mesh::middleware` forces it on every node mutation, so
     a node cannot make the hub expand a template. An operator-created
     `<node>__*` entry that a node writes becomes non-expanding too.
   - **Semantics:** a node's config never draws on the hub's environment.
     Expanding against the node's own environment is a follow-up.

## Boundaries

- The engine's worker listener and the proxy listen on loopback (`mesh`
  writes `host: 127.0.0.1` for the proxy). The gateway listens on the mesh
  only. The front door carries no bus traffic. `mesh` gives the mesh no path
  to the engine except through the gateway. The engine's own listener
  defaults to `0.0.0.0`, which `mesh` does not change; the README tells
  operators to set it to `127.0.0.1`.
- A node:
  - registers only under `<node>::`;
  - calls only its allow list, through `mesh::middleware`;
  - reads only its own configuration entries, never expanded against the
    hub's environment;
  - binds only granted trigger types, never `harness::hook::*`;
  - keeps no grant past a change to it;
  - holds no engine credential.
- Only the hub drives `mesh-node::*` (signed requests), and only the proxy
  drives `mesh::middleware` and `mesh::on-trigger-registration` (live session
  key).
- Removal refuses the node's keys forever and cycles the code. A cycled code
  refuses every old join secret.
- Nodes make outbound connections only to:
  - `public_url`;
  - linked peers and the hub, over UDP;
  - GitHub releases over HTTPS, for `mesh`, the install scripts and worker
    artifacts;
  - wintun.net over HTTPS, on Windows.

  Nothing a node runs as root is fetched over plain HTTP. The binary makes no
  other connection: no log upload, no Tailscale DERP or DNS fallback, no STUN
  unless `stun_port` is set.
- Workers on nodes run as the operator.
- `mesh` changes no DNS settings and no hosts-file line outside its block.

## Dependencies

- **`configuration`** (required): the `mesh` config, the `rbac-proxy` config
  `mesh` owns, and each node worker's `<node>__<worker>` entry.
- **`rbac-proxy`** (required, owned by `mesh`).
- **`ade`** (optional, phase 2): the Console page, which shows nodes, path,
  version, workers and drives, and the join code with its one-liners. Links,
  drives and node workers are edited through the generated config form.
- **Go:**
  - the current `iii` Go SDK (`whatsapp` pins an older one);
  - the `tailscale.com` fork;
  - `filippo.io/cpace`.
- **On nodes:** apt (drive servers) and wintun (Windows).

## File layout

Follow `whatsapp/` (the repo's Go worker) and `docs/sops/go-binary-worker.md`:

- **Top level:** `iii.worker.yaml` (`language: go`, `deploy: binary`,
  `manifest: package.json`), `package.json` (version), `Makefile`
  (`CGO_ENABLED=0 go build -trimpath`), `go.mod`, `main.go`, `NOTICE`.
- **`internal/`:** `{coordinator,meshnet,join,elevate,relay,drives,service,gateway,supervisor,hub}/`.
- **`install/`:** `install.sh` and `install.ps1`, published as release assets.
- **`skills/`:** `SKILL.md`.
- **Also:** `iii-permissions.yaml`, `README.md`, and `ui/` (phase 2).
- **Repo files to touch:**
  - a `.deploy/workers.yaml` entry with `kind: go-binary`, Windows targets,
    and the install scripts as assets;
  - the root `iii-permissions.yaml` rules;
  - the [required changes](#required-changes-outside-mesh).

## Testing

**Unit tests:**
- links → who sees whom;
- device-code parsing (case, hyphen, O/0, I/L/1);
- the rate limit, both with and without `trusted_proxies`;
- tickets: single use and expiry;
- name sanitizing and collisions;
- the middleware's configuration-id rule;
- version comparison → update;
- a drive's desired state per node;
- the shared scoped-id predicate.

**Integration** (Linux CI, no root):
- A hub and two userspace nodes (tsnet) in one test process, with a real
  engine and `rbac-proxy`.
- **Real bootstrap, two namespaces.** A real `ide` boots on a node twice: once
  with the hub in `default`, once with the hub in a project namespace. Each
  time its `configuration::ensure` and `get` on `<node>__ide` succeed in
  `default`, and `<node>::shell::exec` is callable from a harness in the hub's
  namespace.
- Join, then a map without links (the nodes cannot see each other), then with
  a link (they can).
- Removal reaches the peers in under 5 s. The removed node, re-registering
  with fresh keys and its saved join secret, is refused.
- Two nodes each run a fake worker named `echo`, with a trigger type. Both
  appear as `<node>::echo` and answer, and neither displaces the other.
- A node reading `configuration::get` on `mesh`, or binding `configuration`
  to another entry, is refused.
- A loopback client that dials the proxy without a ticket is refused.

**Adversarial** (a malicious node and a malicious agent):
- **Templates:** a node `ensure`s `<node>__probe` with
  `${OPENAI_API_KEY}` as its value. Its `get` returns the literal template.
- **Hooks:** a node binding `harness::hook::pre-trigger` or `post-trigger` is
  refused, and a hub call made afterwards reaches no node.
- **Supervisor reach:** an agent calls `<node>::mesh-node::workers::apply`
  directly, through `fp::pipe` and through `fp::batch`. All three are refused,
  by `fp` and by the node's signature check. A replayed signed request is
  refused.
- **Middleware reach:** an agent calls `mesh::middleware` without a session
  key and is refused.
- **Condition callbacks:** a node registers a `configuration` binding on its
  own entry with `condition_function_id: tailscale::logout` (or any function
  outside its grants), then changes the entry. The binding is refused by both
  `mesh` and `rbac-proxy`, and the function never runs.
- **Name squatting:** node `evil` connects a worker named `ide` while node
  `mac` is offline. It is registered as `rbac-proxy:evil::ide`, and `mac`'s
  `ide` connects normally when `mac` returns.
- **Revocation:** an id is removed from `nodes.<name>.allow`. The next call
  to it from an already-connected node worker is refused within 5 s.
- **Tokens (phase 3):**
  - a removed node re-registering with fresh keys and its preserved single-use
    or reusable token is refused;
  - a consumed single-use token does not join a second machine.

**Machine matrix** (acceptance): a fresh Ubuntu 26.04 VM, a Mac on another
network, Windows 11 (phase 2), and the hub behind a named Cloudflare tunnel.

## Acceptance criteria

1. The one-liner joins a fresh Ubuntu VM and a Mac on another network in under
   a minute each. `mesh::nodes::list` shows both online, and neither machine
   opened an inbound port.
2. `iii trigger mac::shell::exec '{"command":"uname","args":["-a"]}'` returns
   the Mac's kernel. A harness agent asked "what OS does each of my machines
   run?" answers through the skill, with no commands spelled out for it.
   Each remote call goes through approval.
3. Two unlinked nodes cannot reach each other. After `links: [[a, b]]` they
   can, by `<name>.iii.internal`, within 5 s.
4. Removal is fast and final:
   - Within 10 s of `mesh::nodes::remove mac`, its functions are gone from
     `engine::functions::list`, its peers have lost its name, and its TUN,
     hosts block and workers are gone.
   - Its saved state and the old code no longer join.
   - Joining again needs `mesh leave` and the new code.
5. Brute-force and cycle limits hold:
   - After 10 wrong codes from one address within 10 minutes, that address is
     refused until the window ends, and another address still joins.
   - Behind cloudflared, with `trusted_proxies` set, a forged
     `CF-Connecting-IP` from a direct client is ignored.
   - After `mesh::join::cycle` the old code is refused, and joined nodes are
     unaffected.
6. A node's Tailscale keeps working before, during and after join and leave:
   `tailscale status` and MagicDNS names both work.
7. With the hub behind a named Cloudflare tunnel, joins and relayed paths
   work. `mesh status` shows `relay hub` when UDP is blocked and `direct` on
   a LAN. A hub upgrade updates every online node within a minute of it
   reconnecting.
8. On a drive served by the Ubuntu node:
   - a file written on the Mac at `/Volumes/proj` appears on Windows `W:`
     and on Linux `/drives/proj`;
   - an fcntl lock held on one node is seen on another.
9. A node stays inside its grants:
   - A node registering `mesh::nodes::list` gets `<node>::mesh::nodes::list`,
     not the hub's id.
   - Its `configuration::get` on another entry is refused.
   - A `laptop::coder::read-file` call carrying a model-written `fs_scope`
     runs under the laptop's configured roots, through a direct call,
     `fp::pipe` and `fp::batch` alike.
   - It cannot read a hub environment variable through a configuration
     template.
   - It cannot bind a harness hook, or have the engine run a function outside
     its grants as a trigger condition.
   - It cannot take a worker name that belongs to another node.
   - A real `ide` on a node boots and serves with the hub inside and outside
     `default`.
   - It loses a revoked grant within 5 s, without being removed.
   - No agent can drive `mesh-node::*` or `mesh::middleware`, by any path.
10. A capture at the front door shows no plaintext except CPace messages and
    `/id`. No node opens a connection to a Tailscale-run host. No node
    fetches anything executable over plain HTTP.
11. Phase 3:
    - An ephemeral node joined by token, idle for 3 minutes, is gone from the
      mesh.
    - A revoked or expired token no longer joins, and the device code still
      does.
    - A consumed token, or the token of a removed node, never joins again.
    - A function bound to `mesh::node-changed` sees `online` and `offline`
      when a node's service stops and starts.
    - `browser` runs on a node as `<node>::browser::*`.

## Open items

Each of these sets a default list or a follow-up, not the design.

1. **`default_allow` and `default_trigger_types`.** Run `ide` (then
   `browser`, `computer`, `tmux`) behind the gateway and record every call
   and binding each makes. Keep only what passes the confinement test in
   [Gateway](#gateway), and record what each worker loses without the rest,
   as done for `ide`'s turn-log hooks.
2. **Console forms for node workers.** Check whether the Console finds them
   through `<node>::ide::configuration-id`. If it does not, that is a
   follow-up; config stays editable through `configuration::set` on
   `<node>__<worker>`.
3. **`ide` on Windows.** It ships no Windows build today. Phase 2 ships
   Windows nodes either way, and the skill says which machines lack shell and
   file functions.
4. **Console page from Go.** The Console page helper (`iii_console_ui`) is
   Rust. Register `mesh::ui-content` at the protocol level from Go, per
   `docs/sops/injectable-console-ui.md`.
5. **Rust SDK reconnects after a refused registration?** The Go SDK stops
   reconnecting when the engine refuses a registration. Check the Rust SDK,
   because `ide` and most workers on nodes use it. The supervisor's restart
   covers both cases, but the log should say why.
