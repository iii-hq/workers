# quick-tunnel: OpenTunnel provider, and the mesh hub behind a tunnel

> **Note, 2026-11-10.** The install lines mentioned here were removed with the machine node: `mesh::join::code` and `mesh::join::token` no longer return install commands. Join codes, tokens and the public URL are unchanged.

Status: design, ready to implement. Decisions marked **[call]** are the architect's and can be overruled.

## 1. Summary

- `quick-tunnel` gets `provider: cloudflared | opentunnel`. Default `cloudflared`. One provider per worker instance.
- OpenTunnel runs in-process through the `opentunnel` crate (`=0.6.2`). No binary.
- The lease manager, state file, lock, expiry, backoff and the public contract do not change. Only the "start a tunnel and tell me what happens" part moves behind a trait.
- The mesh hub gets `tunnel: {enabled, tunnel_id}`. When enabled and no `public_url` is set, the hub leases a tunnel and uses its URL as the public URL.
- The Mesh home screen gets a switch, "Expose through a tunnel".

## 2. quick-tunnel

### 2.1 Config schema

`src/config.rs`, struct `Config` (still `serde(default, deny_unknown_fields)`). One new field:

```rust
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind { #[default] Cloudflared, Opentunnel }

pub struct Config {
    pub provider: ProviderKind,   // new, default cloudflared
    pub targets: BTreeMap<String, String>,
    pub cloudflared: String,      // unchanged; ignored when provider = opentunnel
    pub state_path: PathBuf,
    // ... all other fields unchanged
}
```

```yaml
provider: opentunnel            # omit for cloudflared
targets:
  webhooks: http://127.0.0.1:3112
  mesh: http://127.0.0.1:49180
```

Stored configs without `provider` load as before. No `opentunnel:` options block: nothing in it is needed **[call]**. The relay API URL is not configurable; tests set it through the Rust constructor (2.6). A second key, `proxy_protocol`, was added by the follow-ups (8.2).

Extra validation when `provider = opentunnel` (in `validate()`):

| Rule | Why |
|---|---|
| every target origin is `http://` | the worker hands plaintext bytes to the target; it cannot speak TLS to it |
| every target id matches `^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?$` | the id is used as the OpenTunnel profile name **[call]**; `webhooks` and `mesh` pass |

The `cloudflared` string is validated as today in both modes (it has a valid default), so the rule set does not branch further.

Also touched: `iii.worker.yaml`, `config.collect.yaml`, the `EntrySpec` description in `main.rs:91`, and `--manifest` (prerequisites line becomes "cloudflared binary, only for provider cloudflared").

### 2.2 Trait and module layout

```
src/
  provider.rs               trait Provider, trait Session, enum Signal, struct Sink, fn build(&Config)
  provider/cloudflared.rs   today's spawn() + read_output() + Process, moved, not rewritten
  provider/opentunnel.rs    new
  parser.rs                 unchanged, except public_url(url, suffix) takes the domain suffix
  prerequisites.rs          unchanged
  manager.rs                calls the trait instead of spawn()/Process
```

```rust
// provider.rs
pub enum Signal {
    Url(String),        // already validated by the provider
    Connected,
    Disconnected,
    Fatal(String),      // sanitized text; the manager calls fail() with it
}

/// Stamps tunnel_id + generation, so a provider cannot mix generations.
#[derive(Clone)]
pub struct Sink { /* tunnel_id, generation, mpsc::Sender<Output> */ }
impl Sink { pub async fn send(&self, signal: Signal); }

#[async_trait::async_trait]
pub trait Provider: Send + Sync {
    /// "cloudflared" | "opentunnel". Prefix of manager-side error strings.
    fn name(&self) -> &'static str;
    /// Must not block. `state_dir` is the parent of state_path.
    fn start(&self, tunnel_id: &str, origin: &str, state_dir: &Path, sink: Sink)
        -> Result<Box<dyn Session>, String>;
    /// status.prerequisites. None = nothing to report.
    async fn prerequisites(&self) -> Option<Prerequisites>;
}

#[async_trait::async_trait]
pub trait Session: Send {
    /// Some(reason) once the tunnel ended by itself. Polled on every tick.
    fn exited(&mut self) -> Option<String>;
    /// Stop and wait. Must leave nothing running.
    async fn stop(self: Box<Self>);
}
```

`async-trait` is already in `Cargo.lock` (0.1.92), so it costs no new download. A trait object is needed, not an enum, because integration tests in `tests/` inject a fake.

Manager changes (`manager.rs`), all mechanical:

- `Manager::open(config)` stays and calls new `Manager::open_with(config, Arc<dyn Provider>)` with `provider::build(&config)`. Tests use `open_with`.
- `Manager` drops the `cloudflared: Arc<str>` and `probe` fields; `status()` calls `provider.prerequisites()`.
- `Tunnel.process: Option<Process>` becomes `Option<Box<dyn Session>>`. `child.try_wait()` becomes `session.exited()`.
- `Output { event: Option<LogEvent>, broken }` becomes `Output { signal: Signal }`. `Signal::Fatal(text)` calls `fail(text)`.
- Manager-owned error strings use the provider name: `format!("{name} startup/reconnect timeout")`, `format!("{name} edge connection interrupted")`. With cloudflared every string is byte-identical to today.
- The READY rule (`connected && candidate.is_some()`), generations, backoff, cooldown, budget reset, persistence: untouched.

The cloudflared provider keeps its own strings (`cloudflared exited (..)`, `cloudflared spawn failed (..)`, `cloudflared log stream invalid or oversized`) and maps `LogEvent` to `Signal` one to one.

### 2.3 OpenTunnel provider lifecycle

One OpenTunnel tunnel identity per target id (profile = target id). Each target has an independent `Tunnel`, alive only while it has leases, exactly like a cloudflared child **[call]**. Not one identity with many routes: target lifetimes are independent, and route changes re-attach the bridge.

`start()` spawns one tokio task and returns a `Session` holding its `JoinHandle` and a shutdown `oneshot`. The task:

1. `client.ensure(profile).await` (creates the identity on first use, else loads it).
2. Validate `https://{identity.hostname}`, read or create the target's route name (8.1, A2), then `sink.send(Url("https://{route}.{hostname}"))`.
3. `client.connect(profile, {route: Route::new("<ip>:<port>")})`, never the root route `@` (8.1, A1). The route target comes from the validated allowlist origin, never from a caller. A target listed in `proxy_protocol` gets `.with_proxy_protocol(Some(V2))` (8.1, B1).
4. Forward events until shutdown.

| OpenTunnel | Signal | Resulting status |
|---|---|---|
| `start()` called | none | `starting` (first attempt) or `reconnecting` (retry) |
| `ensure` Ok | `Url` | still `starting` |
| `Event::Connected` listing the target's route | `Connected` | `ready`, `public_url` set |
| `Event::Disconnected`, `Event::Reconnecting` | `Disconnected` | `reconnecting`, `public_url` null, same generation, deadline armed |
| `Event::Connected` again | `Connected` | `ready`, same generation, same URL |
| `Event::Stopped`, `wait()` returns | `Fatal("opentunnel stopped")` | `reconnecting` then `failed` by the shared retry rules |
| `ensure`/`connect` Err | `Fatal(..)` see 2.5 | same |
| no `Connected` before `startup_timeout_ms` | manager deadline | `opentunnel startup/reconnect timeout` |
| last lease gone, or shutdown | `stop()` | `stopped` |
| `ConnectionOpened/Closed`, `CertificateRenewed`, `Connecting` | ignored | |
| broadcast `Lagged` | re-read `tunnel.status()`: `Connected` state gives `Connected`, else `Disconnected` | |

`stop()`: send shutdown, the task runs `tunnel.close()`; wait up to 2 s, then abort the task (dropping a `Tunnel` stops it). `exited()`: `JoinHandle::is_finished()` gives `Some("opentunnel task ended")`.

The provider never calls `Client::remove`. The identity outlives leases and restarts.

First provisioning waits for an ACME certificate and can exceed the default 30 s `startup_timeout_ms`. The retry then calls `ensure` again, which resumes from `pending.json`, so retries converge. The README recommends `startup_timeout_ms: 120000` with opentunnel. Defaults are not forked per provider **[call]**.

### 2.4 URL shape and state

- URL: `https://<route>.<id>.opentunnel.xyz`. `<id>` is assigned by the relay (12 base32 chars); `<route>` is a random name the worker stores per target (8.1, A1 and A2). The bare `https://<id>.opentunnel.xyz` is not served. Validator: `parser::public_url("https://<id>.opentunnel.xyz", ".opentunnel.xyz")` checks the relay's hostname with the same strict rule as today (https, one DNS label, no port, path, query, userinfo or fragment); the route is checked as one lowercase DNS label.
- **The URL is stable** across generations and restarts while the identity directory exists. With cloudflared it changes on every process. Consumers must keep using `generation`, not the URL, to detect change. The README sentence that promises `.trycloudflare.com` widens to "the domain of the configured provider".
- State directory: `<parent of state_path>/opentunnel/`, default `data/quick-tunnel/opentunnel/`. Created by the worker with mode `0700`; an existing directory with any group/other bit is a startup error (fail closed). Passed as `Storage::at(dir)`. `Storage::xdg()` and `ClientOptions::default()` are never used. The crate writes `<dir>/<profile>/` as `0700` and `token`, `private-key.pem` as `0600`.
- Losing the directory means a new hostname. Back it up with the worker's data.

### 2.5 Errors

Snapshot `error` stays sanitized: fixed strings only. The raw `opentunnel::Error` goes to `tracing::warn!`.

| Cause | `error` |
|---|---|
| `Error::Api{status: 429}` | `opentunnel rate limited` |
| `Error::is_transient()` | `opentunnel relay unreachable` |
| `Error::CertificateFailed` | `opentunnel certificate failed` |
| `Error::Io`, `Error::Corrupt` | `opentunnel state unreadable` |
| any other ensure/connect error | `opentunnel provisioning failed` |
| hostname fails the URL validator | `opentunnel returned an invalid hostname` |
| `Connected` without the target's route | `opentunnel route not attached` |
| the `route` file is unreadable or not a DNS label | `opentunnel state unreadable` |
| tunnel stopped by itself | `opentunnel stopped` |

All of them go through the shared `fail()`: bounded retries, then `failed` with cooldown. No provider-specific retry code.

`status.prerequisites`: unchanged for cloudflared. Omitted for opentunnel (`None`, the field already has `skip_serializing_if`) **[call]**. An old github then shows its "cloudflared" row as `unknown` with the text "update quick-tunnel" (not a blocker: only a definite `missing` refuses). The `status` response now says `provider: "opentunnel"` and a new github shows a correct row (8.1, C1 and C2).

### 2.6 Dependencies and the test seam

```toml
opentunnel = "=0.6.2"      # young crate, pin exactly
async-trait = "0.1"
[dev-dependencies]
rcgen = "0.14"             # self-signed cert for a seeded identity
```

MSRV 1.89 is below the repo toolchain (1.97.1). The lock already has rustls 0.23.46 with ring and no aws-lc, so the crate's "install ring as default provider" does not clash. It adds a second reqwest (0.13 next to 0.12). No cargo feature gate **[call]**.

Two seams, no network:

1. `Manager::open_with(config, Arc<dyn Provider>)` with a scripted `FakeProvider` in `tests/support/`. It records `start`/`stop` calls and lets the test push `Signal`s.
2. `OpenTunnel::new(opentunnel::ClientOptions)` is public. `provider::build` passes `{api: None, storage: Storage::at(dir), ca_cert: None}`. Tests pass `api: http://127.0.0.1:<closed port>` and a temp dir.

No fake relay server is built **[call]**. The bridge handshake itself is the crate's job and has its own tests.

### 2.7 README (security section, added in the README's language)

State plainly:

- With `opentunnel`, **visitor TLS ends inside the quick-tunnel process.** The relay sees only ciphertext, but the worker holds the certificate private key and sees every request in clear text. With cloudflared, TLS ends at Cloudflare and the worker never sees traffic.
- The key, the certificate and the tunnel token are in `data/quick-tunnel/opentunnel/`. Anyone who reads that directory can impersonate the hostname.
- **The local service sees every visitor as `127.0.0.1`** with no forwarding header, unless its target is listed in `proxy_protocol` (8.2): then every connection starts with a PROXY protocol v2 header that carries the visitor's address. A header such as `X-Forwarded-For` or `CF-Connecting-IP` is written by the visitor. Do not trust loopback peers, and authenticate everything exposed.
- The provider runs in the worker's environment. Unlike the cloudflared child (clean env), it honors `HTTPS_PROXY`, `SSL_CERT_FILE` and `OPENTUNNEL_CA_CERT`.
- The relay's hostname `<id>.opentunnel.xyz` is public as soon as its certificate is issued (Certificate Transparency logs), but it does not answer. The served name `<route>.<id>.opentunnel.xyz` is in no certificate. It is hard to guess, not secret: DNS resolvers, the relay, everyone given the URL, and every local worker or agent allowed to call `quick-tunnel::status` see it. Delete the profile's `route` file to change the URL, or the profile directory to change the hostname too.
- The relay is a third-party service (`opentunnel.xyz`) with rate limits (30 new tunnels per hour per IP, 5 certificates per day per tunnel) and no stated SLA.

Also update `skills/SKILL.md` where it says cloudflared is required.

## 3. Mesh hub (Go)

### 3.1 Config

`mesh/internal/hub/config.go`:

```go
type TunnelConfig struct {
    Enabled  bool   `json:"enabled"`
    TunnelID string `json:"tunnel_id,omitempty"` // default "mesh"
}
type Config struct {
    // ...
    Tunnel *TunnelConfig `json:"tunnel,omitempty"`
}
```

```yaml
tunnel:
  enabled: true
  tunnel_id: mesh      # optional
```

`omitempty` pointer, so the schema gains no required field. `Validate()`: `tunnel_id` empty or `^[A-Za-z0-9_:.-]{1,128}$`. `cloneConfig` copies the pointer target. Hub mode only; client mode ignores it.

The operator must list the target in the quick-tunnel config: `mesh: http://127.0.0.1:49180` (the hub `listen` port). The hub never writes another worker's config.

### 3.2 Rule for the public URL **[call]**

```
effective = public_url            if public_url != ""
          = tunnel URL            if tunnel.enabled and the tunnel is ready
          = ""                    otherwise
```

An explicit `public_url` wins, and while it is set the hub holds **no lease** (state `overridden`). Reason: a manual address means the operator already has a front door; a second public entrance nobody asked for is exposure with no use. This also keeps STUN simple: `stun_port > 0` still requires `public_url` (a tunnel cannot carry UDP), unchanged validation.

### 3.3 Changes by file

**`config.go`**: 3.1.

**`tunnel.go` (new)**, about 150 lines:

- Hub fields: `tunnelMu sync.Mutex`, `tunnel tunnelState{leaseID, tunnelID, status, errText string; expires time.Time; observed bool}`, `tunnelURL string` (guarded by `h.mu`), `tunnelKick chan struct{}` (buffer 1).
- `func (h *Hub) publicURL() string`: the rule in 3.2, read under `h.mu`.
- `func (h *Hub) reconcileTunnel(ctx)` under `tunnelMu`:
  1. `want := cfg.Tunnel != nil && cfg.Tunnel.Enabled && cfg.PublicURL == ""`.
  2. Not wanted, or `tunnel_id` changed: release the held lease (best effort), clear state.
  3. Wanted and (no lease, or under 5 min left, or the last `status` did not list our lease): `quick-tunnel::acquire {consumer_id, tunnel_id, expires_at: now+10m}`.
  4. Wanted: `quick-tunnel::status {tunnel_id}`; store `status`, `error`, and the URL only when `status == "ready"`.
  5. After the first successful acquire, once: `client.Observe(iii.Source{FunctionID: "quick-tunnel::on-changed", Namespace: h.Namespace}, "mesh::on-tunnel-change", iii.ObserverOptions{ID: "mesh-tunnel", TargetNamespace: h.Namespace})`. A failure is logged; the tick still covers it.
  6. If the effective URL changed: set `h.tunnelURL`, call `h.Coordinator.Relay(..)` with the same fallback as `applyCoordinator`.
- Call failures: acquire error containing `unauthorized tunnel_id` gives state `unavailable`, error `quick-tunnel has no target "<id>"; add "<id>: http://127.0.0.1:<listen port>" to its targets`. Any other error gives `unavailable`, `quick-tunnel is not installed or not responding`. Never returned as a hub error.
- `func (h *Hub) releaseTunnel()`: `quick-tunnel::release {lease_id}` with a fresh 3 s context.
- Lease: 10 minutes, renewed when under 5 remain **[call]**. A crashed hub stops being exposed within 10 minutes; a renewal is one small fsync in quick-tunnel every 5 minutes.
- `consumer_id = "mesh:" + hex(sha256(namespace + "\x00" + configID))[:32]`, the same trick github uses to stay inside the 128-byte id rule.
- Calls go through the existing `rpc` interface (`Call(ctx, iii.CallRequest)`), so tests use the existing fake.

**`configuration.go`**: register `mesh::on-tunnel-change` next to `mesh::on-config-change` (same internal metadata). The handler does no I/O: it ignores other `tunnel_id`s and does a non-blocking send on `tunnelKick`. `onConfigChange` kicks after a successful `applyConfig` (a kick, not a direct call, because it runs under `h.reload`).

**`hub.go`**: `StatusResult` gains `Tunnel *TunnelStatus json:"tunnel,omitempty"` with `{enabled, tunnel_id, state, url, error}`. The `Run` loop gains `case <-h.tunnelKick:` and calls `reconcileTunnel` on the existing 15 s tick too. `defer h.releaseTunnel()` is placed after `defer h.Close()` so it runs first, while the client is still connected. Skipped in local/interface mode.

`state` values: quick-tunnel's own (`starting`, `ready`, `reconnecting`, `failed`, `stopped`) plus `unavailable` and `overridden`. `tunnel` is omitted when not enabled.

**`functions.go`**: `Status()` returns `PublicURL: h.publicURL()` and fills `Tunnel`. `joinCode` (`:164-171`) and `JoinToken` (`:43`) use `h.publicURL()`.

**`lifecycle.go`**: the join callback (`:92`), the `/join` handler (`:169`) and `applyCoordinator` (`:204`) use `h.publicURL()`. The `public_url is not set` text becomes `public_url is not set and the tunnel is not ready` only when the tunnel is enabled.

**`iii.worker.yaml`**: no dependency on quick-tunnel (same choice as github). **`README.md`**: a short "Expose through a tunnel" section with the target line, the rule in 3.2 and the notes in 3.4.

### 3.4 Consequences to document

- **Use `provider: opentunnel` for the mesh.** Its hostname is stable. With cloudflared the URL changes whenever the tunnel process restarts, including every hub restart (the hub releases its lease on shutdown). Machines already joined and tokens already issued keep the old URL and stop working. The hub does not notify them.
- With opentunnel, every visitor reaches the hub from `127.0.0.1`, and a `CF-Connecting-IP` or `X-Forwarded-For` header is the visitor's own. While its tunnel runs on opentunnel the hub drops both headers from loopback peers, so a loopback entry in `trusted_proxies` has no effect (8.1, B8). All tunnel visitors then share one rate-limit bucket (10 joins per 10 minutes), and anyone who keeps it full blocks every join. List the hub's target in quick-tunnel's `proxy_protocol` to count each visitor separately (8.1, B1 to B6; 8.3).
- With cloudflared the setting is `trusted_proxies: [127.0.0.1/32, ::1/128]`, because cloudflared connects from loopback and Cloudflare sets `CF-Connecting-IP` (8.1, B7).
- The join limit counts an IPv6 visitor by its /64 (8.1, B9).
- Only the front door (`listen`) is exposed, the same surface `public_url` already publishes. The loopback-only admission listener is a separate port and is not routed.

## 4. Mesh UI (`mesh/ui/page.tsx`)

- `Basics` gains `tunnel: boolean`, path `['tunnel', 'enabled']`; `basicsOf` reads it with `at(config, path) === true`.
- The status type gains `public_url?: string` and `tunnel?: {enabled, tunnel_id, state, url, error}`.
- In the hub section's `SettingsList`, above Public URL:
  - `SettingsField` "Expose through a tunnel", description "Gives this hub a public address through the quick-tunnel worker.", control `Switch` bound to `edit('tunnel', next)`.
  - When `data.tunnel` exists, one line under it: `ready` shows the URL with the existing copy control; `starting`/`reconnecting` show "Starting the tunnel…"; `overridden` shows "Not used while a Public URL is set"; `unavailable`/`failed` show a `StatusPanel variant="warn"` with `tunnel.error`.
- Public URL field stays. Description becomes "A manual address. Overrides the tunnel."
- The "needed before machines can join" badge shows only when `!saved.public_url && !saved.tunnel`.
- Save bar: `public_url` and `tunnel` join `share` in the "applied within a few seconds" set. Both are applied live by the hub; the current "after a restart" label is wrong for `public_url`.
- No install button for quick-tunnel.

## 5. Tests

**quick-tunnel** (`cargo test`, no network)

- `tests/process.rs`, `tests/contracts.rs`: pass unchanged. This is the proof the refactor kept cloudflared behavior and strings.
- `src/config.rs`: `provider` defaults to cloudflared; a config without the field loads; opentunnel rejects an `https://` target and a target id that is not a valid profile name; unknown provider value is rejected.
- `tests/provider.rs` with `FakeProvider`: Url then Connected gives `ready`; Connected without Url is not ready; Disconnected clears the URL and keeps the generation; Fatal retries with a new generation and ends `failed`; the same URL in a new generation is accepted; last release calls `stop` exactly once; `start` error is observable in the snapshot; `prerequisites` omitted when the provider returns None.
- `src/provider/opentunnel.rs` unit tests: every `Event` maps to the right `Signal`; every `Error` class maps to its fixed string and never leaks the raw message; hostname validator accepts `abc234.opentunnel.xyz` and rejects other domains, ports and paths.
- `tests/opentunnel.rs`: the state directory is created `0700` under the state path and a group-readable one is refused; with `$HOME`/`XDG_DATA_HOME` pointed at an empty temp dir, nothing is written there; empty storage plus a closed API port gives `opentunnel relay unreachable`; a seeded identity (rcgen self-signed) plus a closed API port emits the URL and never `ready`, ending in the startup timeout; `stop()` returns within the 2 s bound.

**mesh hub** (`go test ./internal/hub/`, fake `rpc`)

- `config_test.go`: tunnel absent, enabled, bad `tunnel_id`; JSON without `tunnel` round-trips with no `tunnel` key; `cloneConfig` does not alias.
- `tunnel_test.go`: enabled with no `public_url` acquires and reads status; `ready` makes `Status().PublicURL`, the join code install lines and a new token carry the tunnel URL; explicit `public_url` wins and no acquire is sent; setting `public_url` later releases the lease; renewal fires under 5 minutes and not before; a lease missing from `status.leases` is re-acquired; `tunnel_id` change releases then acquires; quick-tunnel missing gives `unavailable` and the hub keeps running; unauthorized target gives the fix text; not ready gives the join error; on-changed for another `tunnel_id` does not kick; `releaseTunnel` sends release once.

**mesh UI** (`mesh/ui/tests/render.mjs`)

- Switch renders in hub mode and not in client mode; toggling saves `{tunnel: {enabled: true}}`; `ready` shows the URL; `unavailable` shows the error; `overridden` note; badge hidden when the tunnel is on; save bar text for `tunnel` and `public_url`.

## 6. Out of scope

- A third provider, a self-hosted relay option, or a configurable relay URL.
- A fake relay server and an end-to-end bridge test.
- Rotating or deleting the OpenTunnel identity from a function or the UI.
- Telling joined machines or issued tokens about a changed hub URL.
- Client mode, STUN through a tunnel, auto-adding the `mesh` target to the quick-tunnel config, installing quick-tunnel from the Mesh UI.
- A cargo feature to compile OpenTunnel out.

## 7. Risks to check first while implementing

1. `ensure()` cancel-safety: `stop()` may abort it mid-provision. The crate's `pending.json` resume suggests it is safe; confirm by reading `client.rs` before relying on retries.
2. `opentunnel` 0.6.2 public API was read from docs, not compiled here. Confirm `Event`, `Identity.hostname`, `Storage::at` and `Tunnel::close` signatures on the first build.
3. How long the relay keeps an idle tunnel identity is not documented. If it expires, `ensure` fails and the hostname changes after the operator deletes the profile.
4. The observer `Namespace` for `quick-tunnel::on-changed` (hub namespace assumed) and whether the engine's permission rules let `mesh` call `quick-tunnel::acquire/release`. The 15 s tick makes a failed observer harmless; a denied call shows as `unavailable`.

## 8. Follow-ups: unguessable URL, visitor address, consumers

Status: design, ready to implement. Sections 2.1, 2.3, 2.4, 2.5, 2.7, 3.4 and 6 were corrected in place where this section changes them. Crate references are to `opentunnel` 0.6.2.

### 8.1 Decisions

**A. Unguessable URL**

- **A1. Each target serves one route with a random name, never the root route `@`.** The URL is `https://<route>.<id>.opentunnel.xyz`. Reason: the certificate names `<id>.opentunnel.xyz` and `*.<id>.opentunnel.xyz` (`provision.rs:14`), so Certificate Transparency logs publish `<id>` but never a route name. Without an `@` route the client resets a connection to the bare hostname before any TLS handshake (`tunnel.rs:551-562`), and does the same for any route name it does not serve. The certificate already covers one label under the hostname, so nothing is reissued.
- **A2. The route name is 16 lowercase hex characters, the tail of a v4 UUID (62 random bits).** It is generated once and stored in `<state dir>/opentunnel/<target id>/route`, mode `0600`: written to a temporary file in that directory, synced, then linked without replacing an existing file. An unreadable file, or content that is not one lowercase DNS label, fails the generation with `opentunnel state unreadable`. It is never regenerated silently, because that would change the URL. Reason: `uuid` and `tempfile` are already dependencies; the file is removed together with the profile directory; deleting only `route` changes the URL without a new certificate. A guess costs one connection through the relay.
- **A3. `parser::public_url` does not change.** The provider checks the relay's hostname with it (one label under `.opentunnel.xyz`, as before) and builds the URL from that hostname and the validated route name. cloudflared parsing is untouched.
- **A4. An identity created before this change keeps its hostname and certificate; its URL changes once**, from the bare hostname to the route hostname. Machines joined through the old URL must join again. The old shape was never released.
- **A5. Hard to guess is not secret.** The route name is seen by DNS resolvers, anyone on the network path (the TLS server name is sent in clear), the relay, everyone given the URL (it is in join commands and tokens, and joined machines use it as their relay address), and every local worker or agent allowed to call `quick-tunnel::status` or `mesh::status`. No log line prints it. It takes the target out of what log scanners find. Authentication stays the job of the exposed service.

**B. Visitor address**

- **B1. The operator turns the header on, per target, in the quick-tunnel configuration: `proxy_protocol: [mesh]`.** Not the consumer through `acquire`. Reason: whether a local service parses the header is a fact about that service, like its allowlist entry. A switch a caller controls could turn it on for `webhooks`, whose HTTP listener would then refuse every visitor. And `acquire` refuses unknown fields, so a new hub would fail against an old quick-tunnel.
- **B2. PROXY protocol version 2 only.** Its 12-byte signature cannot start an HTTP request, and the header carries its own length.
- **B3. quick-tunnel reports the mode on the lease: `leases[].proxy_protocol: "v2"` in `status`. The consumer learns it there.** The snapshot (the `acquire` response and `on-changed`) does not carry it, and the mesh hub gets no configuration key. Reason: the lease has to record the mode anyway (B4), and a lease alive in a process always has that process's mode, so a second key on the snapshot would say the same thing twice.
- **B4. A lease records the mode it was granted under. On boot quick-tunnel drops persisted leases whose mode differs from the configured one**, in both directions. The mode is fixed for the life of a quick-tunnel process (configuration applies on restart). After a restart with the other mode the target has no lease, so no tunnel runs until the holder acquires again.
- **B5. Targets that share a port must all be listed in `proxy_protocol` or none.** This gives one mode per port inside one quick-tunnel: no other route of the same worker reaches a listed port without the header. It does not check that the listed target is the consumer's own port. That the `mesh` target points at the hub's `listen` port is the operator's job.
- **B6. The hub reads a header only on the front door, only from a loopback peer, and only while a flag is set.** One thing sets the flag: a `quick-tunnel::status` answer whose entry for the hub's own lease says `proxy_protocol: "v2"`. Every other outcome of a reconcile clears it: tunnel not wanted, no client, failed acquire, failed status, lease not listed, field absent or different, shutdown. An `acquire` that returns a lease id the hub did not hold clears it at once, before the status call: a new id is a new grant, possibly under the other mode. A loopback connection that does not start with the signature passes through unchanged, so a local CLI keeps working while the flag is set.
- **B7. With provider cloudflared, `proxy_protocol` is a configuration error.** The right setting there is the existing one on the hub, `trusted_proxies: [127.0.0.1/32, ::1/128]`: cloudflared connects from loopback and Cloudflare sets `CF-Connecting-IP`. No code is added for it.
- **B8. While its tunnel runs on opentunnel, the hub drops `CF-Connecting-IP` and `X-Forwarded-For` from loopback peers on the front door.** Reason: opentunnel hands the visitor's bytes to loopback unchanged, so those headers are the visitor's, and a loopback entry in `trusted_proxies` (the cloudflared setting) would let each forged header open a fresh rate-limit bucket. The two settings live in two workers' configurations, so a provider switch must not depend on the operator editing the other one. A second flag holds this. `status.provider == "opentunnel"` sets it. A failed quick-tunnel call keeps it. It is cleared by a status that reports another provider, or when the hub wants no tunnel and holds no lease. With the header on (B6) a tunnel connection's peer is the visitor's address, not loopback, so nothing is dropped from it.
- **B9. The join limit counts an IPv6 visitor by its /64.** Reason: with the header on, a visitor that owns a /64 would otherwise have 2^64 buckets of 10 tries against the 40-bit join code without forging anything. `join.ClientAddr` returns the /64 for IPv6 on every path, tunnel or not. IPv4 is unchanged.

**C and D. Consumers**

- **C1. The `status` response gains `provider`, omitted when it is `cloudflared`.** Not the snapshot: github's checklist and the hub both read `status`, and nothing reads a provider from `acquire` or `on-changed`. Omitted for cloudflared so that every byte of a cloudflared deployment's payloads stays as it was; "absent" already has to mean cloudflared for older versions.
- **C2. github:** `provider == "opentunnel"` gives the row `tunnel_provider` in state `ok`. Anything else keeps today's cloudflared logic, including "update quick-tunnel" for a version too old to report the binary.
- **D1. whatsapp accepts `https://<label>.<label>.opentunnel.xyz`** next to the unchanged trycloudflare rule: https, exactly two lowercase DNS labels under `opentunnel.xyz`, no port, path, query, userinfo or fragment. The bare `<id>.opentunnel.xyz` is refused, because quick-tunnel never returns it.
- **D2. The mesh hub and github keep their URL checks.** Neither ever checked the host (https, no credentials, query or fragment), so both accept the new shape and neither gains a host list. Both take the URL only from quick-tunnel, whose own rule is strict.

### 8.2 Wire contract

quick-tunnel configuration, one new key. The default is empty and stored configs without it load.

```yaml
provider: opentunnel
targets:
  webhooks: http://127.0.0.1:3112
  mesh: http://127.0.0.1:49180
proxy_protocol: [mesh]      # targets whose service reads a PROXY v2 header
```

Validation errors: `proxy_protocol needs provider opentunnel and ids listed in targets`, and `targets on the same port must all be in proxy_protocol or none`.

No request changes. The snapshot (`acquire` response, `on-changed` payload) does not change. The `status` response gains two optional keys:

```json
{"tunnel_id":"mesh","status":"ready","public_url":"https://r0ute234abcd5678.abc234.opentunnel.xyz",
 "generation":"<uuid>","error":null,
 "leases":[{"lease_id":"<uuid>","consumer_id":"mesh:<hex>","tunnel_id":"mesh","expires_at":"<rfc3339>","proxy_protocol":"v2"}],
 "provider":"opentunnel"}
```

| Key | Value | Absent means |
|---|---|---|
| `provider`, in `status` | `"opentunnel"` | cloudflared, and every older version |
| `leases[].proxy_protocol`, in `status` and in the state file | `"v2"` | no header is sent to this lease's target |

Consumers compare each value with exact equality and treat anything else as absent. `"v2"` is a promise that holds while the lease is listed: each connection quick-tunnel opens to the lease's target starts with the header below, before any visitor byte.

With provider cloudflared the `acquire`, `status` and `on-changed` payloads and `leases.json` are byte-identical to before. The registered schemas of the configuration and of `status`, the default configuration value and `--manifest` do change under both providers: they gain the optional keys, and the default configuration gains `proxy_protocol: []`.

Header, as the crate writes it (`protocol/proxy.rs`): the signature `0D 0A 0D 0A 00 0D 0A 51 55 49 54 0A`; `21` (a proxied connection) or `20` (visitor address unknown); `11` (TCP over IPv4), `21` (TCP over IPv6) or `00`; the body length as 2 bytes, big-endian; the body: source address, destination address (all zero), source port, destination port 443, then one field `02 <2-byte length> <TLS server name>`. With `20` the body is only that field. The crate writes at most 294 body bytes. Shared test vectors, server name `r0ute234abcd5678.abc234.opentunnel.xyz`:

```
visitor 203.0.113.7:4000, 69 bytes
0d0a0d0a000d0a515549540a21110035cb007107000000000fa001bb020026723075746532333461626364353637382e6162633233342e6f70656e74756e6e656c2e78797a
visitor [2001:db8::1]:443, 93 bytes
0d0a0d0a000d0a515549540a2121004d20010db80000000000000000000000010000000000000000000000000000000001bb01bb020026723075746532333461626364353637382e6162633233342e6f70656e74756e6e656c2e78797a
address unknown, 57 bytes
0d0a0d0a000d0a515549540a20000029020026723075746532333461626364353637382e6162633233342e6f70656e74756e6e656c2e78797a
```

`mesh::status` gains `tunnel.proxy_protocol: "v2"` while the hub's flag (B6) is set. github's second checklist row with `provider: "opentunnel"` is `{"id":"tunnel_provider","title":"tunnel provider","state":"ok","detail":"quick-tunnel uses OpenTunnel; no cloudflared binary is needed."}`.

| quick-tunnel | Consumer | Result |
|---|---|---|
| old | new hub, github, whatsapp | no new keys: the hub never parses a header and trusts loopback as configured, github shows the cloudflared row as today |
| new, cloudflared | old or new | payloads identical to the old version |
| new, opentunnel | old hub | works; URL in the new shape; all visitors share one bucket; a loopback entry in `trusted_proxies` is still honored (the README warning is the only guard) |
| new, opentunnel | old github | cloudflared row `unknown`, not blocking, as before |
| new, opentunnel | old whatsapp | "invalid public HTTPS URL", as before |
| new, opentunnel, target listed | old hub | every tunnel visitor is refused with 400. List a target only when its service reads the header |
| new, opentunnel, target listed | new hub | each visitor has its own rate-limit bucket |

A state file that holds a lease with `proxy_protocol` cannot be loaded by a build without the field. No released build has opentunnel, so no released downgrade reaches it.

### 8.3 Trust boundary for B

The danger is the hub taking a visitor address that the visitor wrote. Each forged address would be a fresh rate-limit bucket against the 40-bit join code, which does not expire. There are two ways in: a PROXY header parsed while quick-tunnel forwards the visitor's bytes as they are, and a `CF-Connecting-IP` header trusted from a loopback peer.

Why the first cannot happen:

1. Only the first bytes of a connection are parsed, and only on a loopback connection to the front door. A header from any other peer reaches the HTTP parser and gets 400.
2. The hub parses only while its flag is set, and only a status answer that lists the hub's own lease as `v2` sets it (B6).
3. quick-tunnel grants a `v2` lease only for a target the operator listed. For such a target the client writes the header right after it connects, before it copies any visitor byte (`tunnel.rs:836-841`). On a tunnel connection the visitor's bytes are never first; a forged header after the real one is a malformed HTTP request.
4. No other route of the same quick-tunnel reaches a listed port without the header (B5). Whether the listed target is the hub's port is the operator's responsibility.
5. The mode cannot change under a lease. It is fixed per process, and a process that starts with the other mode drops the lease (B4). The tunnel serves again only after the hub acquires, and that acquire returns a lease id the hub did not hold, which clears the flag before the new tunnel can serve (B6).

Why the second cannot happen: while the provider is opentunnel the hub drops the forwarding headers of loopback peers before any handler reads them (B8).

| Event | PROXY flag | Tunnel visitors |
|---|---|---|
| header turned on or off, quick-tunnel restarted | lease dropped; cleared by the next reconcile (at the acquire, if a renewal was due), set again (or not) by the status that follows the new acquire | no tunnel for up to two reconcile ticks (30 s) |
| quick-tunnel stops answering | cleared at the first failed call, although the URL is kept | 400 until a status call succeeds |
| lease expired or released, `public_url` set, tunnel disabled, `tunnel_id` changed | cleared | tunnel stopped |
| hub restarted | starts cleared | 400 until the first reconcile, which runs at start |
| ordinary renewal (same lease id) | unchanged | served |

What stays open, and the READMEs say so:

- While the flag is set, any local process that connects to the front door can state a visitor address. So can another forwarder that passes raw bytes to that port, such as an SSH remote forward or a container port mapping that makes outside peers look like loopback. The sender is trusted by position, as with every PROXY protocol receiver.
- The address is the one the relay reports. A relay that lied could spread guesses over many addresses.
- A second consumer leasing the hub's target could start the tunnel in the new mode before the hub notices a restart (up to 15 s). The hub's target should have one consumer.
- An attacker that holds many /64 prefixes (a /48 is 65,536 of them) still has that many buckets. So does one with many IPv4 addresses.
- A hub that restarts while a lease it held before a crash keeps an opentunnel tunnel up trusts loopback as configured until its first status call succeeds. That call runs at start.
- An old hub behind a new opentunnel quick-tunnel has neither flag.

### 8.4 Tests

**quick-tunnel** (`cargo test`, no network)

- `src/config.rs`: `proxy_protocol_needs_opentunnel_listed_targets_and_one_mode_per_port`.
- `src/provider/opentunnel.rs`: `every_event_maps_to_its_signal` changes, because the attached route is now the target's and `@` alone is "not attached"; new `route_name_is_created_once_private_and_stable`, `an_invalid_route_file_is_an_error_not_a_new_name`, `one_named_route_carries_the_header_only_when_listed`, `proxy_header_matches_the_vector_the_mesh_hub_parses`.
- `tests/opentunnel.rs`: `seeded_identity_emits_the_url_never_connects_and_stops_within_the_bound` changes its expected URL to `https://<route>.abc234.opentunnel.xyz`, with `<route>` read from the file the provider created (16 hex characters, mode `0600`). Stability across starts is covered by the unit test, because every start of this suite costs the crate's 16 s retry.
- `tests/provider.rs`: `a_lease_records_proxy_protocol_and_a_mode_change_drops_it`, `cloudflared_json_has_no_new_keys`.
- `tests/process.rs` and `tests/contracts.rs` pass with no assertion change. `tests/process.rs` gains `proxy_protocol: None` in its two `Lease` literals, which would not compile otherwise.

**mesh** (`go test ./internal/hub/ ./internal/join/`)

- `join/join_test.go`: `TestClientAddrTrustsOnlyListedProxies` gains IPv6 rows (two addresses of one /64 give one key, another /64 gives another, a trusted `CF-Connecting-IP` in IPv6 is masked too, an IPv4-mapped peer stays IPv4). Its existing "IPv6 peer" row changes its expected value from `2001:db8::1` to `2001:db8::`, because the key is now the /64 (B9).
- `hub/proxyproto_test.go`: `TestProxyHeaderGivesVisitorAddress` (the IPv4 and IPv6 vectors), `TestProxyHeaderOptionalOnLoopback` (a plain request and an address-unknown header keep the loopback address), `TestProxyHeaderIgnoredWhenUntrusted` (flag off gives 400 and the handler never runs; a peer that is not loopback is not wrapped), `TestProxyHeaderMalformedClosesConnection`, `TestProxyHeaderIsReadOnce` (a second header is not parsed).
- `hub/tunnel_test.go`: `TestTunnelProxyProtocolFlag`, one case per outcome in B6, including a renewal that keeps the lease id (flag stays set during the status call) and one that returns a new id (flag already cleared during the status call). `TestTunnelRawFollowsProvider`, the rules of B8.
- `hub/lifecycle_test.go`: `TestFrontDoorBehindProxyProtocol`. A loopback forwarder prepends a header to each connection. Through it, the eleventh `POST /join` from one visitor address gets 429 while another address still passes, and a full join plus a control connection work, which proves the upgraded connections survive the wrapper. `TestOpenTunnelIgnoresLoopbackForwardingHeaders`: with loopback in `trusted_proxies`, eleven posts with eleven `CF-Connecting-IP` values pass; once the B8 flag is set, the eleventh post gets 429 whatever the header says.

**github** (`cargo test`, `tests/setup.rs`): `opentunnel_provider_replaces_the_cloudflared_row`. The existing "no prerequisites, no provider" case stays as the old-version proof.

**whatsapp** (`go test -run Tunnel .`): `TestDevelopmentTunnelAcceptsOpenTunnelURL`; the invalid-URL loop gains the bare hostname, three labels, a look-alike suffix, a port, a path, `http://` and an uppercase label, all under `opentunnel.xyz`.

**mesh UI**: no change.

### 8.5 Out of scope

- PROXY protocol with cloudflared, version 1, or a mode requested through `acquire`.
- `provider` or `proxy_protocol` on the snapshot (`acquire`, `on-changed`).
- Host allowlists in the mesh hub and github URL checks.
- A hint on the Mesh page, or in the hub's "no target" text, that the target is not listed in `proxy_protocol`.
- A cap on the number of rate-limit buckets, or a wider IPv6 mask than /64.
- A fake relay and an end-to-end bridge test (still 6). The crate tests that the header precedes the payload; the shared vectors pin the bytes.
- Rotating the route name from a function or the UI.
- whatsapp's "check the cloudflared connection" failure text under opentunnel.

### 8.6 Not verified

1. The relay side of A was read only from the client source: that `<route>.<id>.opentunnel.xyz` resolves, and what the relay answers on the bare hostname before it reaches the client. Two certificates already issued on this machine do carry the wildcard name. Check once by hand against the real relay before release; no test may do it.
2. That the relay accepts IPv6 visitors, and the exact form of the peer string it reports.
3. That the hub's upgraded connections (`/ts2021`, `/derp`) work through the wrapped connection. Their handlers take a generic `net.Conn`; `TestFrontDoorBehindProxyProtocol` is the check.
4. That Cloudflare always overwrites a visitor-written `CF-Connecting-IP` on a Quick Tunnel (the cloudflared setting in B7 relies on it).
