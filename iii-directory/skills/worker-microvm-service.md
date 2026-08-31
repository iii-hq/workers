---
title: Exposing a service that runs inside a worker microVM
type: how-to
description: >-
  Locally-added workers run in libkrun microVMs whose network is egress-only: the
  guest cannot reach its own TCP loopback and the host cannot dial in at all.
  This is the verified networking map plus the guest-initiated tunnel pattern for
  publishing any in-VM HTTP/WebSocket service to a browser, and the console
  injectable-UI contract for embedding it.
---

# Exposing a service that runs inside a worker microVM

When a worker runs a long-lived server of its own — an editor, a dev server, a
notebook, a database console, a preview renderer — the hard part is not starting
it, it is *reaching* it. A locally-added worker runs inside a libkrun microVM with
**egress-only** networking, so most of the obvious approaches fail silently. Read
the map below before writing any code that binds a port inside a worker.

## 1. Is this worker in a VM?

`engine::workers::info { name }` → `isolation: "libkrun"` with `os: "linux … (arm64)"`
means yes, whatever the host OS is. Workers added from a local project directory
(`worker::add { source: { kind: "local" } }`) and other managed Node/Python workers
get a VM; the always-on Rust workers (console, state, shell, cron, …) run natively
on the host. Consequences that bite immediately:

* `process.platform` / `process.arch` are the **guest's** — on an Apple-silicon host
  a worker must download `linux-arm64` artifacts, not `darwin-arm64`;
* `process.cwd()` is `/workspace` (the worker's own source, bind-mounted from the
  host — writes there are real host edits and trip the source watcher), `$HOME` is
  `/`, and everything else the worker writes lands in the VM's overlay;
* whatever the worker installs is installed **in the VM** — usually the point:
  heavy, untrusted, or OS-specific payloads never touch the host.

## 2. The networking map (verified, not assumed)

The launcher (`iii worker __vm-boot --network`) runs a **userspace smoltcp stack on
the host** that proxies guest TCP **outward only**. The guest learns its addressing
from env vars: `III_INIT_IP` (e.g. `100.96.0.2`), `III_INIT_GW` (`100.96.0.1`),
`III_INIT_CIDR` (`30`), `III_INIT_DNS`.

| from → to | works? | notes |
| --- | --- | --- |
| guest → internet (`curl`, `wget`) | **yes** | DNS via `/etc/resolv.conf` = the gateway |
| guest → **gateway** `$III_INIT_GW:PORT` | **yes** | this *is* the host's `127.0.0.1:PORT` — the only route to the host |
| guest → `127.0.0.1` (its own loopback) | **no** | there is no `lo` route; packets exit via `eth0` and time out |
| guest → its own `III_INIT_IP` | **no** | the userspace stack does not hairpin |
| host → guest, any address/port | **no** | no route, no ARP entry, no port publishing — `--network` is egress-only |
| guest → engine `ws://localhost:49134` | yes | pre-wired; the engine sees the worker as `127.0.0.1` |
| host → guest via `iii worker exec` | yes | multiplexed virtio-console channel, not TCP — the only ingress that exists |

Two traps worth real time:

1. **A worker cannot health-check its own service over TCP.** Binding `0.0.0.0:PORT`
   and probing `127.0.0.1:PORT` times out even though `/proc/net/tcp` shows the
   listener. Put the service on a **unix socket** and probe with
   `curl --unix-socket <sock> http://localhost/`.
2. **Node's global `fetch` (undici) is unreliable in-guest**, failing with
   `UND_ERR_CONNECT_TIMEOUT` on routes where `curl` succeeds. Shell out to
   `curl`/`wget` for anything load-bearing; keep `fetch` as a fallback only. For
   download progress, poll `fs.stat(dest).size` against a
   `curl -sSIL <url> | grep -i content-length` probe rather than streaming.

## 3. The pattern: a guest-initiated tunnel

Since ingress does not exist, **the guest must dial out** and the pairing happens on
the host. This shape carries HTTP, static assets *and* WebSocket upgrades
(verified `101 Switching Protocols`), which is why it succeeds where an
HTTP-function proxy cannot — a browser WebSocket cannot be served through function
calls, and a Service Worker cannot intercept WS.

```
browser ──▶ host 127.0.0.1:PUBLIC ─┐
                                   ├─ paired by a tiny host byte-pump
guest   ──▶ host 127.0.0.1:TUNNEL ─┘
  (pool of idle outbound sockets, dialed at $III_INIT_GW:TUNNEL)
                     │
                     ▼
     unix socket ──▶ the real service, inside the VM
```

1. **Service on a unix socket** in the VM (most servers support this: VS Code's
   `--socket-path`, node/express `listen(path)`, gunicorn `--bind unix:…`).
2. **Host byte-pump**: two loopback listeners — `PUBLIC` for browsers, `TUNNEL` for
   the guest. It copies bytes only: no application code, no state, no filesystem
   access. Keep it small enough to audit at a glance.
3. **Guest keeps a pool** (≈8) of connections to `$III_INIT_GW:TUNNEL`, each opening
   with a handshake line (`MAGIC <secret>\n`). The pump parks them idle; per browser
   connection it pops one, writes `GO\n`, and pipes both directions. The guest end,
   on `GO`, connects to the unix socket and pipes — **forwarding any bytes that
   arrived in the same chunk after `GO\n`** (the classic prefix-buffer bug) — then
   opens a replacement idle socket.
4. Pool hygiene: `setNoDelay` + `setKeepAlive` both ends, ≈1.5s backoff on failed
   dials so a dead pump does not spin, and have the pump wait a few seconds for a
   fresh tunnel socket instead of dropping a browser connection when the pool is dry.
5. Health is **two probes**: service-up (`curl --unix-socket`) and end-to-end
   (`curl http://$III_INIT_GW:PUBLIC/` from the guest, which traverses pump +
   tunnel + socket). Only report "running" when the second one passes — that is what
   the browser will actually experience.

### Bootstrapping the host side from inside the VM

The worker installs and launches its own host helper over the bus — no hardcoded
host paths, no manual setup step, no separate install instructions:

```js
// write it: stdin carries the source, bash writes it on the HOST
await worker.trigger({ function_id: "shell::exec", payload: {
  command: "bash",
  args: ["-lc", `PID=$(cat ${PIDFILE} 2>/dev/null || true); [ -n "$PID" ] && kill "$PID" 2>/dev/null; cat > ${REMOTE}`],
  stdin: await fs.readFile(pumpSourceInsideTheVM, "utf8"),
}});
// run it: exec_bg outlives the call; env carries ports + a per-boot secret
const { job_id } = await worker.trigger({ function_id: "shell::exec_bg", payload: {
  command: "bash", args: ["-lc", `exec node ${REMOTE}`],
  env: { PUBLIC_PORT: "3210", TUNNEL_PORT: "3211", SECRET: secret, PIDFILE },
}});
```

* use `bash -lc "exec node …"` so a login shell resolves the interpreter on the host;
* keep a **pidfile** and kill the previous pid on start — `shell::exec_bg` jobs are
  children of the shell worker and die with it, so start must be idempotent and
  survivable across worker restarts;
* bind the pump to **loopback only** and treat the handshake secret as per-boot;
* the pump is the one host-side artifact. Be explicit about it: the payload stays in
  the VM, but a byte-relay on the host is unavoidable while ingress does not exist.

## 4. Debugging playbook

* `iii worker exec <worker> --no-tty -- /usr/bin/curl -sS -m 4 http://$GW:PORT/` — a
  shell **inside** the VM; the only host→guest path.
* `cat /proc/net/route` (only `eth0` rows ⇒ no loopback) and `cat /proc/net/tcp`
  (hex ports — `0C8A` = 3210, state `0A` = LISTEN) when `ip`/`ss` are missing from
  the image.
* `worker::status { name }` → `stderr_tail` carries VM boot + dependency install;
  the boot line also reveals mounts (`/mnt/host-src`, `/opt/iii`) and the erofs
  overlay. `worker::logs` for more.
* Ship a `<worker>::diag` function returning guest addressing, routes, DNS, egress
  status, socket listing and both probes. Cheaper than guessing, and it keeps the
  VM's internals observable from the bus.
* Prove the round trip with the *service's own* logs (e.g. a remote-agent server
  logging a management/extension-host connection) rather than a screenshot.

## 5. Embedding it in the console

A worker ships console UI as assets; no console rebuild:

* register `console:script` with `config { path: "<worker>/page.js" }` and
  `console:style` with `"<worker>/styles.css"`. The trigger's `function_id` is a
  content function returning `{ content, content_type }` — read from disk per call
  and the dev loop is edit + reload the tab. Re-registering a path overrides it.
* the script is **ESM with `export default function setup(host)`**, optionally
  returning a cleanup function. Bare imports resolve through the console's static
  import map — `react`, `react-dom`, `react-dom/client`, `react/jsx-runtime`,
  `@iii-dev/console-ui` — so hand-written `createElement` needs **no bundler**.
* `host` = `{ iii: { trigger(fnId, payload), on, registerTrigger, browserId,
  addConnectionStateListener }, components, useTheme, path, pages, functionTriggers,
  configForms }`.
* `host.pages.register({ id, title, render })` → a page routed at `#/ext/<id>`;
  `host.configForms.register(configId, Component)` overrides a worker's config form;
  `host.functionTriggers.register(renderer)` adds trigger-specific UI.
* verify with `console::ui-manifest` (paths, hashes, style-lint warnings, per-worker
  enable state); users can disable a worker's UI via the console config
  (`injectableUi.disabledWorkers`).
* an `<iframe>` to the pump's loopback port works: the console sets no restrictive
  CSP, so all that matters is the embedded service not sending `X-Frame-Options` /
  `frame-ancestors`. Check with `curl -D -` before designing around it.
* prefix every CSS class and keep selectors scoped — injected styles are global.
