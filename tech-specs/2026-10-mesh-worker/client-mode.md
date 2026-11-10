# Mesh client mode: design

> **Updated on 2026-11-10.** The machine node this design refers to was removed, with its installer, CLI, service, self-update, supervisor, drives and links. Client mode is now the only way to join a hub. Passages about `internal/cli`, `internal/elevate`, `Updater`, `DisableUpdate`, `UnsupportedDrives`, `mesh-node::update` and `mesh-node::drives::apply` describe code that no longer exists. The hub now sends its version in the `mesh-node::status` request and hands the namespace with `mesh-node::workers::apply`.

Paths are relative to the repository root unless a prefix says otherwise.
- `sdk:` is `$GOMODCACHE/github.com/iii-hq/iii/sdk/packages/go/iii@v0.26.0-rc.5`
- `ts:` is `tailscale.com@v1.102.4`
- `compose:` is `crates/iii-compose/src` in the iii repository

I re-read these citations for this design: `mesh/main.go`, `mesh/internal/hub/{hub,config,configuration,ui,auth,nodecalls}.go`, `mesh/internal/node/{node,runtime,functions}.go`, `mesh/internal/meshnet/mesh.go`, `mesh/internal/coordinator/userspace.go`, `mesh/internal/cli/{join,state,node}.go`, `sdk:client.go`, `compose:spawn.rs` and `compose:lifecycle.rs`. The rbac-proxy, coordinator and engine citations come from the reader findings and were not re-read.

## 1. Intent

The mesh worker gets a third role. A normal iii worker inside a local project (for example the harness on `ws://127.0.0.1:49335`) joins a remote hub as its own mesh node, with no root access. It then publishes the functions of the local workers the operator picks to the hub project as `<node>::<fn>`, and forwards each call back to the local engine. The running hub at 192.0.2.10 (version 0.1.0) needs no change.

## 2. Goals and non-goals

**Goals (MVP)**
1. `mode: client` in the mesh configuration entry. When it is absent the mode is `hub`, so today's behaviour does not change.
2. No root anywhere: no TUN, no `/etc/hosts`, no launchd or systemd, no socket under `/var/run`. It must run next to the existing `andersons-mbp` root daemon on the same Mac.
3. Selection is by **worker**: `share: [worker, ...]`. The default is an empty list, which shares nothing.
4. Bridging is **request/response only**. Error codes pass through, and descriptions and schemas are copied.
5. Sync: workers or functions that appear or disappear locally show up on, or leave, the hub within about 5 s. Changes to the share list apply without a restart.
6. The node answers the hub's signed `mesh-node::*` control calls. It refuses `update` and never replaces its own binary.
7. The ADE Mesh page shows client status, and the configuration form can edit the client settings.

**Non-goals (pushed back, can come later)**
- Channel splicing. A payload that carries a channel ref is refused with a clear error. Reason: refs only work on the engine that created them (`sdk:channels.go:364`, findings).
- Mirroring observers, triggers or event sources to the hub. Hub observers only see calls that go through the hub.
- Choosing individual functions. Only whole workers are selectable.
- Local workers calling hub functions. Today they only see the local engine.
- Hub-managed workers or drives on a client node. Requests for them get a per-item error.
- Self-update, a leave button, and switching mode while running. A config change outside `share` shows "restart required" instead.
- Checkbox UI built from `engine::workers::list`. v1 uses the existing `StringList` control (`mesh/ui/page.tsx:154-164`).
- Windows client mode, cancellation propagation, and payloads over 16 MiB.
- One remote connection per shared worker. All bridged functions go on one connection. On the hub they appear under one proxied worker, `rbac-proxy:<node>::mesh-bridge` (`rbac.rs:637-656`, findings).

## 3. Architecture

```
 CLIENT MACHINE (no root)                                     HUB HOST 192.0.2.10   
 +---------------- local project, engine ws://127.0.0.1:49335 ---+   +------------- hub project --------------+
 | queue  state  ide  mesh-probe ...  (local workers)            |   | engine <-- rbac-proxy :49200           |
 |        ^                                                      |   |   ^         (adds/strips <node>::)     |
 |        | local.Call(fn)  (caller identity = "mesh")           |   |   |              ^                     |
 | +------+----------- mesh worker, mode: client --------------+ |   |   |         gateway :49135             |
 | | local client "mesh" -----------------------> engine :49335| |   |   |   (tailnet-only listener, admits   |
 | |                                                           | |   |   |    by source tailnet IP)           |
 | | Bridge (implements node.WorkerController)                 | |   | hub mesh worker:                       |
 | |   remote client "mesh-bridge" --+                         | |   |  front door :49180 (/join /ts2021 /derp)|
 | | node.Node (signed mesh-node::*)  |                        | |   |  coordinator, OnNodeOnline reconciler   |
 | |   remote client "mesh-node" ----+-> ws://127.0.0.1:<fwd>  | |   +------------------^---------------------+
 | |                                  v                        | |                      |
 | | loopback forwarder -> netstack.DialContextTCP(hub6, 49135)| |   WireGuard over UDP |(LAN) or DERP
 | | userspace wgengine + netstack (UDP port 0, inbound denied)|-+----------------------+
 | | shim 127.0.0.1:0  (control+DERP over WS to public_url)    | |
 | +-----------------------------------------------------------+ |
 +---------------------------------------------------------------+
 state: $III_COMPOSE_DIR/data/mesh-client/{join.json, mesh/state, mesh/removed}
```

Data path for a hub caller of `harness-mbp::state::get`:
1. The hub engine routes the call to the node's `mesh-bridge` session.
2. rbac-proxy strips the prefix (`interceptor.rs:331`).
3. The bridge handler for `state::get` runs `local.Call`.
4. The result returns the same way.

## 4. Mode selection

- **Config field, not a flag.** Compose starts the binary with no arguments. With no subcommand, main only parses `--url/--local-config/--collect-interface` and calls `hub.Run` (`mesh/main.go:34-55`).
- **Add to `hub.Config`** (`mesh/internal/hub/config.go:17-27`):
  `Mode string json:"mode,omitempty" jsonschema:"enum=hub,enum=client"` and `Client *client.Config json:"client,omitempty"`.
  - Both must be `omitempty`. The schema sent to `configuration::ensure` (`configuration.go:35-45`) marks non-omitempty fields as required (invopop `reflect.go:104-107`, findings).
  - Entries that already exist have no `mode` key, and the decoder rejects unknown fields (`config.go:127-128`).
  - `Validate` rejects any mode other than `""`, `hub` or `client`, and calls `Client.Validate()` when the mode is client.
- **Branch point:** in `hub.Run`, right after `ensureConfig` (`mesh/internal/hub/hub.go:61-66`) and before `Start`. Only the hub path calls `Start`, so client mode never binds `0.0.0.0:49180` (`config.go:38`) and never creates hub state under `data/mesh` (`lifecycle.go:81-83`). The branch does four things:
  1. Calls `h.registerConfiguration(ctx, client, true)`. With `local=true` this registers only `mesh::configuration-id` and `mesh::on-config-change`, with no observers (`configuration.go:119-136`). `page.tsx:255` needs `mesh::configuration-id`.
  2. Calls `h.registerUI(client)` (`ui.go:34-44`).
  3. Calls `os.Unsetenv("III_WORKER_NAME")`. Compose always sets it (`compose:spawn.rs:241`), and the SDK applies it over `WithName` on every client built afterwards (`sdk:client.go:417-421`). Without the unset, both remote clients would be named `mesh`. In the same namespace that is `WORKER_NAMESPACE_CONFLICT`, which is fatal. The local client has already captured its name, so unsetting is safe.
  4. Returns `client.Run(...)`.
  It skips `registerNodeEvents`, `registerFunctions`, `registerProxyFunctions`, the 15 s proxy ticker and `startNodeCalls` (`hub.go:72-103`), so client mode never touches rbac-proxy.
- **Seeding:** the compose entry sets `config_override: {mode: client}`. Compose merges the override and `set_config`s it at `compose:lifecycle.rs:1542-1551`, inside `resolve_config`, which runs at `:1077`, before the spawn plan at `:1116`. So the very first boot already sees `mode: client`. Override only `mode`, so the ADE can still edit `client.*`.
- **Mode change at runtime:** not applied. The config poll (section 7) notices it and `mesh::status` reports `restart required`.

## 5. Enrollment and state

- **Directory:** `$III_COMPOSE_DIR/data/mesh-client`, mode 0700. It is a sibling of the hub's `data/mesh` (`lifecycle.go:81-83`).
- **Files:**
  - `join.json` (0600): `{public_url, secret, hub_identity, requested_name}`.
  - `mesh/state`: the node key, through `store.NewFileStore` (`meshnet/mesh.go:181`).
  - `mesh/removed`: the marker written by the watcher (`mesh.go:211-214`).
- **First boot, no `join.json`:**
  1. If `client.join` is empty, register `mesh::status` (`state: "waiting for join"`) and keep polling the config (section 7). Do not crash-loop.
  2. Resolve `client.join` through `secrets::resolve {"ref": "<secret://… | env://…>"}`. The response is `{"value": "..."}` (`crates/judge-provider/src/secrets.rs:152-167`). The caller is `mesh`, so the secret's consumers must include `mesh`.
  3. Run `cli.ParseJoin([]string{hub_url, value})` (`mesh/internal/cli/join.go:22`). It accepts an `iiimj_` token, which carries `PublicURL`, or a `XXXX-XXXX` code plus a URL.
  4. Follow the same order as `cli/join.go:115-135`: `join.DecodeToken` or `join.Redeem` (CPace over public `/join`), then `join.FetchIdentity` and `Identity.Verify(pin)`. Skip the `exists`, `install` and elevation steps.
  5. `requested_name` is `client.name` if set, otherwise `requestedName(hostname)` (`cli/join.go:155-175`). Export that function or copy its 20 lines.
  6. Write `join.json`. Use `cli.State` if it is exported; if not, a 4-field struct plus `os.WriteFile(…, 0600)`.
- **Later boots:** load `join.json`. The node key in `mesh/state` authenticates. The join secret was only needed for the first registration: a code-derived secret stops working when the code rotates, and a token is single use (findings: `coordinator.go:484-525`, `join/code.go:60-65`).
- **Operator:** `elevate.Operator()` (`mesh/internal/elevate/operator_unix.go:33`). `node.New` and the hub both require a non-empty operator (`node/node.go:78`, `hub/nodecalls.go:58`).
- **Name:** the hub de-duplicates names. With `name` unset on this Mac you get `andersons-mbp-2` (`coordinator/names.go:30-70`, findings). Read the assigned name live from `Mesh.SelfName()`; it becomes the `<node>::` prefix. Persisting it is optional, since the daemon only does that for display (`cli/node.go:67-73`).
- **Removal by the hub:** `node.Run` returns `ErrRemoved` (`node/runtime.go:78-79`) after the watcher writes `removed` (`mesh.go:201-220`). Client mode then idles with status `removed by hub; delete data/mesh-client to rejoin`.

## 6. Networking (userspace)

**Why a separate node is needed.** The gateway listens only on the hub's tailnet IPv6 netstack listener (`coordinator/userspace.go:177-186`). It identifies the caller only by source address: `lookup(peer.Addr())`, refusing unknown peers and `hub` (`mesh/internal/gateway/gateway.go:47-60`). It then mints its own ticket. So the client must be a real tailnet node.

Without root, that means wgengine with a fake TUN plus a gVisor netstack. Dialing `hub.iii.internal` through the OS on this Mac would leave through the daemon's TUN and log in as `andersons-mbp`.

**Changes in `meshnet`:**
1. In `startMesh` (`mesh/internal/meshnet/mesh.go:127`), when `dev == nil` (today: "nil device selects upstream's userspace TUN", `:125-126`), also build a netstack. Copy from `coordinator/userspace.go:125-142`:
   - After `NewUserspaceEngine` (`mesh.go:173`), call `netstack.Create(logf, m.sys.Tun.Get(), m.engine, m.sys.MagicSock.Get(), m.dialer, m.sys.DNSManager.Get(), m.sys.ProxyMapper())` (`ts:netstack.go:333`) and `m.sys.Set(stack)`.
   - Set `ProcessLocalIPs = true`.
   - Install deny-all `GetTCPHandlerForFlow` and `GetUDPHandlerForFlow`. **This is a security requirement, not hardening.** Without it, netstack forwards inbound tailnet TCP to `127.0.0.1:<port>` (`ts:netstack.go:1676-1688,1727-1728`, findings). Every peer could then reach the local engine on `:49335`.
   - After `NewLocalBackend` (`:190`), call `stack.Start(lb)`. Close the stack in `release()` (`:275-293`).
   - Set the keepalive envknobs as `userspace.go:101-102` does.
   - The MSS-clamp hooks (`mesh.go:187-188`) use the other wrapper hook pair, so they do not collide (findings, `ts:netstack.go:443-444`).
2. **Hosts file guard:** skip `WriteHostsBlock` when `m.hostsFile == ""` (`mesh.go:206`, `:272`). Today an empty path fails `os.ReadFile` (`meshnet/hosts.go:74-77`).
3. Add exported `UpUserspace(dir, publicURL, secret, name string, pinned key.MachinePublic) (*Mesh, error)`, which is `NewShim` + `startMesh(dir, shim, secret, name, 0, nil, nil, "")`.
   - It does not call `alone()`, `tstun.New`, `osRouter` or `safesocket.Listen` (`mesh.go:99-121`).
   - UDP port 0 avoids 41643, which the daemon holds (`mesh.go:94`).
   - The shim listens on loopback only and bridges `/ts2021` and `/derp` to `public_url` over WebSocket (`shim.go:65-75,138-159`, findings).
4. Add `(*Mesh).Forward() (string, error)`:
   - Listen on `127.0.0.1:0`.
   - For each accepted connection, resolve the target at that moment: `Peer("hub")` (`mesh.go:246-255`) plus `GatewayPort()`. Dial it with `stack.DialContextTCP` (`ts:netstack.go:948`) and copy bytes both ways.
   - Return `ws://127.0.0.1:<port>`.
   - Because the target is resolved per connection, the URL never changes; after a gateway restart the SDK reconnects to the new port.
   - About 40 lines.

**Why a forwarder.** The Go SDK has no dialer or `http.Client` option. Every dial is `websocket.Dial(…, &websocket.DialOptions{HTTPHeader: …})` (`sdk:client.go:1185`), which falls back to `http.DefaultClient` (`coder/websocket dial.go:75-76`). Channel URLs are built from `Address()`, so they would use the forwarder as well.

Alternatives I rejected:
- Swapping `http.DefaultClient`'s transport process-wide. The local client is already dialing before the mode is known (`hub.go:49-53`), so this is a data race.
- An SDK `WithHTTPClient` option. That is the clean long-term fix, but it means an SDK release.

**Hub address family.** `Peer` returns `Addresses().At(0)`, and the gateway listens on `tcp6` only. Assert in a test that the address is IPv6, and prefer the first IPv6 address in `Forward`.

## 7. Sharing and the bridge

**Where it plugs in: the existing `node.WorkerController` seam** (`mesh/internal/node/node.go:35-44`). The daemon's supervisor gets `SetRuntime(Runtime{Node, Namespace, URL})` and starts workers that dial `URL` in `Namespace` (`supervisor/supervisor.go:56,86-108`). The Bridge does the same, except that its "workers" are proxies of local workers. This gives it, with no new plumbing:
- **The right namespace.** `Runtime.Namespace` is `h.Namespace`, taken from the signed `workers::apply` (`hub/nodecalls.go:116`, `node/functions.go:129-137`). The live hub's `III_NAMESPACE` is unknown, and it no longer matters.
- **The lifecycle.** An empty URL suspends, and a name or URL change restarts (`node/runtime.go:43-50`, `node.go:103-113`).
- **Hub UI.** `Status()` feeds `status.workers` (`node/functions.go:76-91`, `hub/nodecalls.go:67-69`).

**Bridge methods:**
- `SetRuntime(ctx, rt)`. **It must be a no-op when `rt` is unchanged**, the same as `supervisor.go:97-99`. The hub runs `queueNode` on every gateway auth (`hub/auth.go:76`), which leads to `workers::apply`, then `setRuntime`. A non-idempotent SetRuntime would reconnect, re-auth and loop forever.
  - When `rt` changes: close the old remote client, which drops its refs.
  - If the URL is non-empty: build `iii.New(rt.URL, WithOtel(disabled), WithName("mesh-bridge"), WithNamespace(rt.Namespace), WithHeaders(http.Header{"Authorization": nil}))`, register the current desired set, and start `go Connect`.
  - The `Authorization: nil` header stops the SDK from adding `Bearer $III_TOKEN` (`sdk:client.go:334-350`), which would leak the local engine token to the hub's rbac-proxy and `mesh::auth`.
- `Apply(ctx, workers)` returns `WorkerResult{Name, Error: "client-mode node does not run hub-managed workers"}` for each item. When the list is empty it returns `[]`, which is today's case: no `nodes.<node>.workers` entry (`nodecalls.go:119`).
- `Status()` returns one `WorkerStatus` per share entry. State is `shared`, or `not running` when no local functions are found for that worker.

**Discovery.** Every 5 s, and once at start:
1. `local.Call("engine::functions::list", {"workers": share})`. Rows are `{function_id, namespace, worker_name, description?, metadata?}`, and internal functions are excluded by default (`engine_fn/mod.rs:101-122,250-266`, findings).
2. Keep rows where `namespace` equals the local namespace (`hub.go:45-48`) and the id does not start with `engine::`, `mesh::` or `mesh-node::`.
3. For ids that are new to the bridge, make one batched `engine::functions::info {"function_ids": [...]}` call to copy `request_schema` and `response_schema` (`engine_fn/mod.rs:125-147,292-309`, findings).

I chose polling over observing `engine::on-functions-changed`. That event has no catch-up snapshot and would need a re-list after every reconnect anyway (findings: `harness/tests/integration/src/discovery.rs:3-6`), so one ticker covers start, missed events and reconnects. In code: `// shortcut: 5s poll; observe engine::on-functions-changed if lag matters`.

The same tick re-reads the configuration through a `Fetch` closure that `hub.Run` passes in, wrapping `fetchConfig` (`configuration.go:34-88`), so the client package does not import hub.
- A changed `share` is applied in place.
- Any other change to `client.*` or `mode` sets status to `restart required`.

**Registration on the hub.** Diff the desired set against `refs`:
- New ids: `remote.RegisterFunction(id, handler, RegisterFunctionOptions{Description, Metadata, RequestFormat, ResponseFormat})` (`sdk:client.go:524`). Registering before `Connect` is fine, because the registry is replayed on every reconnect (findings, `client.go:583-642`).
- Ids that are gone: `ref.Unregister()`.
- Changed schemas for an existing id are not re-registered in v1.

**Prefixes.** The bridge registers the bare local id. rbac-proxy prefixes it with `<node>::` (`interceptor.rs:322-327,587-597`, findings), because `mesh::auth` returns `FunctionRegistrationPrefix: node` (`hub/auth.go:79`). The hub has no registration hook (`proxy_runtime.go:66-70`, findings). Any id is allowed, in the namespaces `h.Namespace` and `default` (`auth.go:77-78`).

**Call forwarding (one handler per id):**
```go
func(ctx context.Context, data json.RawMessage) (any, error) {
    if refs, err := iii.ExtractChannelRefs(data); err == nil && len(refs) > 0 {
        return nil, &iii.InvocationError{Code: "channels_unsupported", Message: "mesh client bridges request/response only"}
    }
    // shortcut: fixed 10m (hub node calls use 610s, nodecalls.go:36); hub-side cancel does not propagate
    return local.Call(ctx, iii.CallRequest{FunctionID: id, Namespace: localNS, Payload: data, Timeout: 10 * time.Minute})
}
```
- `Call` returns `json.RawMessage`, which goes back unmarshalled.
- A remote `*InvocationError` keeps its code (`sdk:errors.go:40`, `client.go:1801-1812`, findings).
- Trace context goes through `ctx` (findings, `client.go:903`).

## 8. Hub interaction

`node.Node` is reused unchanged, apart from two new options (section 12, step 2). The hub calls `<node>::mesh-node::<fn>` in namespace `default`, signed, with a 5 s timeout for `status` and 610 s for the rest (`hub/nodecalls.go:17-41`). Signatures are still checked (`node/functions.go:101-105`).

| Call | Client answer | Hub requirement |
|---|---|---|
| `mesh-node::status` | `node.Status()` with `Target = HostPlatform().Target`, `Version` = binary version, `Workers` = shared workers | Name must equal the authenticated name; os, arch, version and operator must be non-empty (`nodecalls.go:51-60`) |
| `mesh-node::update` | **Refused** with "client-mode node does not self-update; upgrade the project's mesh worker" | Otherwise `Updater.Apply` replaces `os.Executable()` (`node.go:85-95`, `functions.go:114-123`) |
| `mesh-node::workers::apply` | Sets the namespace, calls `Bridge.SetRuntime`, returns `[]` or per-worker errors | Sent only after status succeeds and the versions match (`nodecalls.go:99-139`) |
| `mesh-node::drives::apply` | Existing nil-drives path: every drive is `unavailable` (`functions.go:160-167`) | Empty unless the hub config puts this node in a drive (`nodecalls.go:152`) |

**Version handling: the client is version-locked to the hub.**
- The hub compares strings exactly: `status.Version != h.Version` leads to `update` and returns before `workers::apply` (`nodecalls.go:107-114`).
- Consequence: **sharing starts only when the versions are equal.** On a mismatch, the hub retries every second, logging only when the error message changes (`nodecalls_runtime.go:97-141`, findings). The client keeps refusing, and the hub UI shows its status, so the problem is visible.
- Build with `make build`, which sets `-X main.version=$(package.json)` (`mesh/Makefile:3,10`). That gives 0.1.0, the same as the live hub.
- A plain `go build` leaves the version empty, which fails `node.New` (`node.go:78`).
- **Do not bump `mesh/package.json` before the live hub runs the same version.** If you must, build with `make build VERSION=0.1.0` for the live test.

## 9. Config schema

The value of the project's mesh configuration entry (id from `III_CONFIG_NAME`, `hub.go:112-117`):

```yaml
mode: client                        # seeded by compose config_override; absent = hub
client:
  hub_url: http://192.0.2.10:49180      # needed only with a join code; a token carries public_url
  join: secret://MESH_JOIN              # REQUIRED form: secret://NAME or env://NAME (never a literal)
  name: harness-mbp                     # requested; the hub may answer harness-mbp-2
  share: [mesh-probe]                   # local worker names (compose container keys); [] = share nothing
# hub fields (public_url, listen, gateway_port, ...) keep their defaults and are ignored in client mode
```

```go
type Config struct {
    HubURL string   `json:"hub_url,omitempty"`
    Join   string   `json:"join,omitempty"`
    Name   string   `json:"name,omitempty"`
    Share  []string `json:"share,omitempty"`
}
```

`Validate()` checks:
- `hub_url` with `validURL` (`hub/config.go:102-121`).
- `join` must start with `secret://` or `env://`.
- `name` must match `^[a-z][a-z0-9-]{0,30}$` (`config.go:42`).
- `share` entries must match `^[a-z0-9-]+$`. Refuse `mesh`, `secrets`, `configuration` and `rbac-proxy` (section 11).

Compose entry for the harness, following the `template/harness/worker-compose.yaml` style:

```yaml
  mesh:
    worker: path://../../mesh
    start_after: [secrets]
    config_override: { mode: client }
    scripts: { pre_run: make build, run: ./bin/mesh }
```

`bin/` is gitignored (`mesh/.gitignore:1`), so `pre_run` builds it with the version ldflags.

## 10. UI changes (`mesh/ui/page.tsx`, rebuilt with `make ui`)

- **Client `mesh::status`.** In client mode the client registers `mesh::status`, returning `{mode: "client", hub_url, node, namespace, state, error, shared: [{worker, functions: ["<node>::fn"]}]}`. The hub's `mesh::status` (`hub/functions.go:265`) and its `StatusResult` (`hub.go:22-31`) stay as they are; a missing `mode` means hub.
- **Page.** Add `mesh::status` to the existing `Promise.allSettled` (`page.tsx:44-47`). When `mode === 'client'`, render one section instead of Machines and Join (`:64-99`): "Connected to <hub_url> as <node>", the state or error, and the shared workers with their hub function ids.
- **Form.** In `MeshConfiguration` (`:220-252`), add a Mode select at the top.
  - For client, show `Field` for hub_url and name, and `StringList` for share (`:145-164`). Add a warning line: "Shared workers can be called by anyone in the hub project, with this project's `mesh` identity."
  - For join, use the Console's `host.components?.SecretKeyField` (name `MESH_JOIN`, consumers `['mesh']`), as `judge-openai/ui/page.tsx:6-9` does. This needs `host` in the form; register it with a closure in `setup` (`page.tsx:254-256`, as `storage/ui/page.tsx:14` does). If the component is missing, fall back to `Field`.
  - Hub fields render unchanged when the mode is not client.

## 11. Security

1. **Share nothing by default.** `share` is empty, and selection is explicit by worker name.
2. **Denylist (config validation):**
   - Workers `mesh`, `secrets`, `configuration` and `rbac-proxy`.
   - Function prefixes `engine::`, `mesh::` and `mesh-node::`, refused at registration.
   - Reason: bridged calls reach the local engine as caller `mesh`. Sharing `secrets` would let the hub project read every secret whose consumers include `mesh`, including `MESH_JOIN` (`secrets/README.md:169-178`, findings).
3. **Exec-capable workers are allowed but carry a warning.** `ide` registers `shell::*` and `coder::*` (`template/harness/worker-compose.yaml:75`). Sharing it gives remote code execution on this machine to the whole hub project. The UI warns, and the operator decides.
4. **Inbound netstack is denied by default** (section 6.1). This is the most important line in the networking change, and it gets its own test.
5. **Loopback forwarder.** Any local process that reaches `127.0.0.1:<fwd>` is admitted to the hub as this node (`gateway.go:56-60`). That is the same trust model as any process on a kernel-TUN daemon host. Mitigations: a random port, loopback only. Upgrade path: an SDK `WithHTTPClient` option; then delete the forwarder.
6. **Tokens and secrets:**
   - `Authorization: nil` on both remote clients stops the `III_TOKEN` leak (`sdk:client.go:334-350`).
   - The join value lives only behind a `secret://` or `env://` ref; config files are versioned (`secrets/README.md:108-116`, findings).
   - `join.json` is 0600 in a 0700 directory.
7. **No self-update.** `update` is always refused, so the project's binary is never replaced.
8. **Hub signatures still verified** for every `mesh-node::*` call (`node/functions.go:101-105`).

## 12. Hub changes required: none

Each requirement is met by existing hub behaviour:
- **Admission:** any authorized tailnet node other than `hub` is admitted by source IP (`gateway.go:47-60`). Nothing checks for a TUN or root (findings, `coordinator.go:785-793`).
- **Enrollment:** public `/join` with CPace, and `/ts2021` and `/derp` through the shim (`join/redeem.go:111-153`, `shim.go:138-159`, findings). The only limit is 10 attempts per address per 10 minutes (`hub/lifecycle.go:85`).
- **Registration of arbitrary ids:** prefixed with `<node>::`, no hook, namespaces `h.Namespace` and `default` (`auth.go:76-79`, `proxy_runtime.go:66-70`, `rbac.rs:684-689`, findings).
- **Undeclared node:** a node with no `nodes.<name>` entry gets an empty `workers::apply` (`nodecalls.go:116-119`). `drives::apply` is empty when the node is in no drive (`coordinator/drives.go:78-115`, findings).
- **UI:** worker names come from the node's own status (`nodecalls.go:67-69`). No hub config is needed.
- **Optional, not MVP:** in `OnNodeOnline`, skip `update` and `apply` for nodes that report `mode: "client"`. The status decode at `nodecalls.go:48` already ignores extra fields. This would remove the version lock. Do it only once clients and hubs are expected to run different versions.
- **Do not** add hub config keys such as `nodes.X.mode`. The live hub's decoder would reject them (`config.go:127-128`).

## 13. Testing

**Unit tests (`go test ./...`)**
- `meshnet/mesh_test.go`: extend the nil-device test (`mesh_test.go:39`, which uses `startMesh(..., nil, nil, ...)`) to dial a test gateway through `Forward()`. Assert that the gateway sees the client's tailnet IP. `hub/gateway_test.go:61-80` already builds the hub side.
- **Security test:** from the hub peer, dial `<client tailnet IP>:<port of a 127.0.0.1 test listener>`. It must be refused, and the listener must never accept.
- `Peer("hub")` returns an IPv6 address. An empty `hostsFile` writes nothing and `Close` returns nil.
- `node`: the `Options.URL` override is used by both `runtime.go:35` and `node.go:112`. With `DisableUpdate`, `mesh-node::update` returns an error and the binary is unchanged. `Authorization: nil` means no bearer header is sent while `III_TOKEN` is set.
- `hub/config`: an old stored value without `mode` parses as hub. `mode: bogus` is rejected. `share: [secrets]` and a literal `join` are rejected. The schema has no new required fields.
- `client/bridge`, with a fake local client:
  - add, remove and namespace filtering from list rows;
  - denied prefixes;
  - **SetRuntime with an identical runtime is a no-op**;
  - an empty URL suspends;
  - forwarding keeps the payload bytes and an `InvocationError` code;
  - a payload with a channel ref gets `channels_unsupported`.

**End to end (`mesh/tests/e2e/client_test.go`, run via `run-tests.sh` with a real `III_BIN`)**
1. `newFixture` gives a hub stack (`fixture_test.go:151`).
2. Start a second engine as the "local project" with a probe worker that registers `probe::echo` and `probe::fail`.
3. Run `client.Run` with `share: [probe]` against the fixture's public URL and a join code.
4. Assert, from a hub-project client:
   - `<node>::probe::echo` round-trips;
   - `<node>::probe::fail` keeps its code;
   - `mesh::nodes::list` shows the node with worker `probe`/`shared` and the hub's version;
   - removing `probe` from `share` unregisters it within about 6 s;
   - a client-reported version mismatch leaves nothing bridged and keeps the binary untouched.

**Live check: harness on `:49335` against 192.0.2.10 (manual; nothing changes on the hub)**
1. Build the client with `make build` and confirm `version == 0.1.0`.
2. Get the join code from the hub's Mesh page (`mesh::join::code`). Store it as vault secret `MESH_JOIN` with consumers `[mesh]`.
3. Add the compose entry from section 9 to the harness project. Set `client.name: harness-mbp`, `client.join: secret://MESH_JOIN`, `share: []`, and start it.
4. Check locally:
   - `mesh::status` shows connected as `harness-mbp` (or `-2`), with the hub namespace;
   - `data/mesh-client/` exists with 0700 and 0600 permissions;
   - `/etc/hosts` is unchanged;
   - no new root processes;
   - `andersons-mbp` is still online on the hub.
5. Start a throwaway probe worker on `:49335` (name `mesh-probe`, function `mesh-probe::echo`), then set `share: [mesh-probe]`.
6. On the hub, from its ADE Functions page or `iii trigger` on the hub host (exact syntax not verified):
   - `harness-mbp::mesh-probe::echo` exists and echoes;
   - an error path keeps its code;
   - the Mesh page shows `harness-mbp`, version 0.1.0, and worker `mesh-probe: shared`.
7. Set `share: []`. The function disappears within about 5 s.
8. Restart the mesh worker. It keeps the same node name, with no re-enrollment.
9. Optionally, remove the node in the hub UI. The client status becomes `removed`.

## 14. Implementation plan (in order)

Total: roughly 600 lines of production code and 500 lines of tests.

| # | Task | Files | Size |
|---|---|---|---|
| 1 | Netstack when `dev == nil` with deny-all inbound and keepalive knobs; hosts-file guard; `UpUserspace`; `Forward()` | `mesh/internal/meshnet/mesh.go`, new `mesh/internal/meshnet/forward.go`, `mesh/internal/meshnet/mesh_test.go` | +80 / +80 tests |
| 2 | `node.Options.URL` (overrides `GatewayURL` at `runtime.go:35`, `node.go:112`); `Options.DisableUpdate` (no BinaryPath lookup or Updater, `update` refuses); `WithHeaders(Authorization: nil)` at `runtime.go:54` | `mesh/internal/node/{node,runtime,functions}.go` + tests | +15 / +40 |
| 3 | Export `requestedName`, or copy it; nothing else in `cli` changes (`ParseJoin` is already exported) | `mesh/internal/cli/join.go` | +2 |
| 4 | `client.Config` and `Validate`; `hub.Config.Mode` and `Client`; validation wiring | new `mesh/internal/client/config.go`, `mesh/internal/hub/config.go` + tests | +60 / +50 |
| 5 | `client.Run`: state load and enroll, `UpUserspace`, `Forward`, `node.New(...)`, 5 s tick (config and functions), `mesh::status`, removed and idle states | new `mesh/internal/client/client.go` | +170 |
| 6 | Bridge: `WorkerController`, discovery and diff, forwarding handler, denylist | new `mesh/internal/client/bridge.go` + `bridge_test.go` | +140 / +150 |
| 7 | `hub.Run` branch: configuration-id, UI, `Unsetenv`, `client.Run` | `mesh/internal/hub/hub.go` | +15 |
| 8 | UI: client status section; mode select and client fields with `SecretKeyField`; `make ui` | `mesh/ui/page.tsx`, embedded assets | +80 |
| 9 | E2E client test | new `mesh/tests/e2e/client_test.go` | +150 |
| 10 | README "Client mode" section; harness compose snippet as a commented example | `mesh/README.md` | +40 |
| 11 | Live check (section 13) before any PR | none | manual |

## 15. Open risks

1. **Version lock.** Sharing works only while the client version equals the hub version (`nodecalls.go:107-114`). After a hub upgrade, clients stop sharing until they are upgraded, and the hub retries every second. The fix is the optional hub change in section 12.
2. **Live hub may differ from this branch.** Everything here comes from source; the running 0.1.0 binary was not queried. Its `III_NAMESPACE` is unknown, but the bridge learns it from `workers::apply`.
3. **Forwarder trust:** any local process can act as this node toward the hub (section 11.5).
4. **Not exercised yet:** two tailscale engines on one Mac (the root daemon and the client) with separate state and UDP ports. A join from this Mac also shares the per-address join budget with the daemon (10 attempts per 10 minutes).
5. **Reconcile errors:** if hub config ever lists workers or drives for the client node, `OnNodeOnline` returns errors and retries every second (`nodecalls.go:140-167`).
6. **Unsupported call semantics:** a hub caller's timeout does not cancel the local call. Channels are refused. Payloads over 16 MiB through the gateway are untested (rbac-proxy lacks `large-messages.v1`, findings). Per-caller RBAC is collapsed to `mesh` on the local engine.
7. **rbac-proxy is installed in client projects anyway.** `iii.worker.yaml:11-13` declares it, and in client mode it is idle. Whether compose resolves manifest dependencies for `path://` entries is not verified.
8. **Process-global env change:** `os.Unsetenv("III_WORKER_NAME")` affects only clients built later. Each SDK client also writes an `iii ps` marker (`process_marker.go:14-15,31`, findings), so `iii ps` may show extra entries.
9. **Copied metadata and schemas:** these are not re-synced when they change for an existing id.

Not checked: the rbac-proxy, engine_fn and coordinator line references (taken from the reader findings), the `iii trigger` syntax, and whether compose restarts a worker that exits.

The main risk is the version lock: if `mesh/package.json` is bumped before the live hub is upgraded, the client never starts sharing.
## 16. Review corrections (binding; they override the sections above)

Two independent reviews checked this design against the source. Apply all of these:

1. **Empty share publishes nothing.** `engine::functions::list` treats an empty or null `workers` filter as "no filter" (engine `engine_fn/mod.rs:1914`). When `share` is empty the desired set is empty and the bridge makes **no** list call. Test it.
2. **Token joins.** If `client.join` resolves to a value starting with `iiimj_`, call `cli.ParseJoin([]string{"--token", v})`; otherwise `cli.ParseJoin([]string{hub_url, v})` (`cli/join.go:29-76`, `join/token.go:66,78`).
3. **Join attempts are scarce.** The hub allows 10 `/join` per address per 10 minutes, successes included (`hub/lifecycle.go:85`, `join/handler.go:89`). Try redeem+register once per distinct resolved join value; on failure set an error status and wait until the value changes. `client.Run` never returns an error that makes compose restart it in a loop.
4. **`functions::info` batches of at most 32 ids**, always with `namespace: <local namespace>` (`engine_fn/mod.rs:150-152,791-808`).
5. **Bridged functions are not observable.** Register every bridged function with `Observable: new(bool)` (false) — the engine list/info calls do not expose the source flag, and rbac-proxy treats a missing flag as observable.
6. **No hub retry loops.** `Bridge.Apply` returns results with an empty `Error`, and the node gets a drive controller that reports drives as unsupported without an error; the reason is reported through `Status()` (`state: "unsupported"`). Otherwise a `"*"` drive or a `nodes.<client>.workers` entry makes the hub retry every second (`nodecalls.go:140-167`, `nodecalls_runtime.go:126-139`).
7. **Honest status.** Report a share entry as `shared` only when the remote client is connected and its functions are registered, otherwise `pending` (or `not running` when the local worker has no functions). `Status()` reads a cached snapshot and never calls the local engine (hub status timeout is 5 s). When `update` is refused, record `hub wants version X` in the local `mesh::status`.
8. **Function-prefix denylist at registration**, in addition to the worker denylist: `engine::`, `mesh::`, `mesh-node::`, `secrets::`, `configuration::`, `rbac-proxy::`. Also refuse bridged ids that contain no `::`.
9. **Channel refs are refused in both directions:** run `iii.ExtractChannelRefs` on the request and on the result.
10. **`Options.URL` keeps the readiness gate** (`node/runtime.go:31-36`: `named && advertised`); it only replaces the address that is handed out.
11. **Simplifications:** a node always has exactly one IPv6 address, so drop the "prefer IPv6" logic. `UpUserspace` must repeat the node-name check that lives in `UpWithPort` (`meshnet/mesh.go:96`).

Scope for this pass: tasks 1-8 and 10 of section 14, with unit tests. The e2e client test (task 9) is deferred; the live check against 192.0.2.10 is the acceptance test.
