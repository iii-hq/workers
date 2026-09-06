# tailscale

Tailscale as an iii worker. It drives the `tailscale` CLI on the Console host and exposes the whole client surface as typed `tailscale::*` functions: connectivity and peers, exit nodes and preferences, publishing local services or the Console to the tailnet with [Serve](https://tailscale.com/kb/1312/serve) or to the internet with [Funnel](https://tailscale.com/kb/1223/funnel), Taildrop file transfer, HTTPS certificates, Taildrive, accounts, tailnet lock, and updates. Every result is structured with keys and secrets stripped, every change to the network is approval-gated, and a **Tailscale** page in the Console puts the everyday actions one click away with QR codes for links.

## Install

```bash
iii trigger compose::add worker=tailscale
```

`iii trigger compose::add` declares the worker in `worker-compose.yaml` and starts it as part of the Compose project. The host needs [Tailscale](https://tailscale.com/download) installed; most functions also need the node signed in (`tailscale::login` returns the sign-in URL, `tailscale::connect` brings it up). Publishing needs MagicDNS and HTTPS certificates enabled for the tailnet; Funnel additionally needs a one-time tailnet-admin approval.

## Quickstart

Open **Tailscale** from the Console navigation or press `⌘K` and run `Open Tailscale`. The page shows the connection, the devices on your tailnet, network diagnostics, preferences, Taildrop, and publishing; its `⌘K` rows refresh, create a link, copy it, open it, and stop the route.

From a function call, ask the node what it sees:

```bash
iii trigger tailscale::status
iii trigger tailscale::peers::list online_only=true
iii trigger tailscale::ping target=phone count=3
```

```json
{
  "target": "phone",
  "direct": true,
  "replies": [
    { "via": "derp", "latency_ms": 41.2, "line": "pong from phone (100.64.0.2) via DERP(nyc) in 41.2ms" },
    { "via": "direct", "latency_ms": 3.4, "line": "pong from phone (100.64.0.2) via 192.0.2.7:41641 in 3.4ms" }
  ],
  "raw": "…"
}
```

Publish the Console to your own devices, then send a file to your phone:

```bash
iii trigger tailscale::share mode=serve https_port=443 path=/
iii trigger tailscale::file::send --json '{"paths":["/Users/me/report.pdf"],"target":"phone"}'
```

`tailscale::share` returns the HTTPS link and its QR code; `mode=funnel` publishes to the internet and needs `allow_funnel: true` in the configuration plus `confirm_public: true` on the request. `tailscale::serve::add` publishes any local port, loopback URL, or directory the same way. Stopping a Funnel route (`share::stop` / `serve::remove` with `mode=funnel`) removes public access and keeps the tailnet route; `mode=serve` removes the route. The worker never resets routes it did not create unless `serve::reset` is called with `confirm=true`.

The full catalogue lives in [`skills/SKILL.md`](https://github.com/iii-hq/workers/blob/main/tailscale/skills/SKILL.md) and in `iii worker info tailscale`.

## Configuration

Settings live in the `configuration` worker under the id `tailscale`; edit them in the Console's global Settings modal and the worker reloads without a restart. An optional `--config <file>` YAML seed is used only when the entry is first created.

```yaml
tailscale_binary: tailscale          # CLI name or absolute path
console_url: http://127.0.0.1:3113   # loopback Console root that tailscale::share publishes
default_https_port: 443              # port used when a publish request omits one
allow_funnel: false                  # operator lock for public Funnel routes
command_timeout_ms: 20000            # per CLI invocation
```

## Run from source with compose

Workers in this repository run locally through [`iii compose`](https://github.com/iii-hq/workers/blob/main/harness/DEVELOPMENT.md). Add a container after the console:

```yaml
containers:
  tailscale:
    worker: path://../tailscale
    start_after:
      - console
    environment:
      RUST_LOG: info
    scripts:
      run: cargo run --locked --bin tailscale
```

The first build runs `pnpm install && pnpm build` inside `ui/` (Node 22 on PATH); set `SKIP_UI_BUILD=1` to reuse an existing `ui/dist`. `III_TAILSCALE_UI_WATCH=1` hot-reloads the page from `ui/dist` into open Console tabs while `pnpm --dir ui watch` runs.

## Security

- Read-only functions (status, peers, netcheck, ping, whois, DNS, preferences, route list, Taildrop targets, lock status, accounts, metrics) are allowed for agents by default. Connect, login, logout, publishing, route removal, preference changes, exit node, Taildrop, Taildrive, certificates, account switch, and update need approval.
- Serve routes are reachable only by devices your tailnet policy admits and carry `Tailscale-User-Login` / `Tailscale-User-Name` headers. Funnel routes are public and carry no identity headers; both locks (`allow_funnel` and `confirm_public`) are required and the page confirms before publishing.
- `serve::add` targets must be a local port, a loopback URL, or an absolute path; the Console target for `share` must be a loopback URL pointing at the Console root.
- Responses never include node keys, private keys, capability maps, or login secrets; `tailscale debug prefs` is read for preferences and its `Config` block is dropped.
