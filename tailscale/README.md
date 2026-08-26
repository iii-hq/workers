# tailscale

Remote access to the local iii Console over your tailnet. The worker drives the installed `tailscale` CLI to publish the Console with [Tailscale Serve](https://tailscale.com/kb/1312/serve) (reachable only by devices in your tailnet, with Tailscale identity headers on every request) or, after two explicit opt-ins, [Tailscale Funnel](https://tailscale.com/kb/1223/funnel) (reachable from the public internet). It ships a **Tailscale** page for the Console that shows connection health, creates the link, renders it as a QR code for a phone, and lists the active routes. Node keys, user records, and capability maps never leave the CLI.

## Install

```bash
iii worker add tailscale
```

`iii worker add` fetches the binary, writes a config block into `~/.iii/config.yaml`, and the engine starts the worker the next time it boots. The host needs [Tailscale](https://tailscale.com/download) installed and signed in (`tailscale status` reports `Running`), MagicDNS and HTTPS certificates enabled for the tailnet, and the Console listening on the configured loopback URL (default `http://127.0.0.1:3113`).

## Quickstart

Open **Tailscale** from the Console navigation or press `⌘K` and run `Open Tailscale`. Pick **Tailnet only**, keep port `443` and path `/`, and create the link: the page shows `https://<node>.<tailnet>.ts.net/` with a QR code. Scan it from a phone that is on the tailnet and the Console opens there. Page commands: `N` creates the link, `C` copies it, `X` stops the route, `R` refreshes.

The same route from a function call:

```bash
iii trigger tailscale::share mode=serve https_port=443 path=/
```

```json
{
  "stage": "ready",
  "mode": "serve",
  "public": false,
  "url": "https://rohits-macbook-pro.tail19ec5c.ts.net/",
  "qr_svg": "<svg …>",
  "authorization_url": null,
  "target": "http://127.0.0.1:3113",
  "https_port": 443,
  "path": "/"
}
```

`tailscale::status` reports connectivity, the MagicDNS name, health notices, whether Funnel is allowed for this node, and every active route with its URL. `tailscale::share::stop` removes exactly one route by mode, port, and path; the worker never runs `serve reset` or `funnel reset`, so routes it did not create stay intact.

### Public access with Funnel

Funnel is off until two locks open: `allow_funnel: true` in the worker configuration, and `confirm_public: true` on the request (the page asks for confirmation before sending it). Funnel supports HTTPS ports 443, 8443, and 10000. If the tailnet policy has not enabled Funnel for this node, `tailscale::share` returns `stage: "authorization_required"` with the Tailscale authorization page as the URL and QR code; approve it, then share again. A Funnel route is reachable by anyone with the link and carries no Tailscale identity headers, so the Console cannot tell who is connecting.

## Configuration

Settings live in the `configuration` worker under the id `tailscale`; the Console's configuration panel edits them and the worker reloads without a restart. An optional `--config <file>` YAML seed is used only when the entry is first created.

```yaml
tailscale_binary: tailscale          # CLI name or absolute path
console_url: http://127.0.0.1:3113   # loopback Console root the routes proxy to
default_https_port: 443              # port used when a share request omits one
allow_funnel: false                  # operator lock for public Funnel shares
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

- Serve routes are only reachable by devices your tailnet policy admits, and Tailscale adds `Tailscale-User-Login` and `Tailscale-User-Name` headers to each proxied request.
- Funnel routes are public. Both locks (`allow_funnel` and `confirm_public`) are required, and the page confirms before publishing.
- The upstream target must be a loopback HTTP(S) URL pointing at the Console root; anything else is rejected at configuration time.
- `tailscale::status` and `tailscale::configuration` are allowed for agents by default; `tailscale::share` and `tailscale::share::stop` need approval.
