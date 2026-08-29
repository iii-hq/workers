# Aspire Dashboard worker

Microsoft Aspire Dashboard as an iii worker. The worker runs the standalone
Aspire Dashboard as a local process (via the Aspire CLI, no Docker), registers
an injectable Console page at `#/ext/aspire-dashboard`, and exposes helpers to
point the engine's built-in `iii-observability` OTLP exporter at it.

This worker is for live display, not long-term observability storage. It leaves
longer-lived trace/log storage in `iii-observability` by using `both` for traces
and logs when it auto-configures that worker.

> Current `iii-observability` caveat: metrics cannot currently be exported to
> OTLP and kept in the local metric memory store at the same time. Its schema
> supports `metrics_exporter: "memory"` or `"otlp"`, but not `"both"`. The
> Console page therefore has a safe default button for traces/logs and a
> separate metrics button that clearly switches metrics to OTLP.

## Quickstart

```bash
pnpm install
pnpm --dir aspire-dashboard build
pnpm --dir aspire-dashboard start
```

The default runtime spawns `npx -y @microsoft/aspire-cli dashboard run` (no
Docker, no manual install — `npx` ships with Node, which this worker already
requires); override `aspire_command` to point at a locally installed `aspire`
binary instead.

Default addresses. Every port is configurable, so read the live values from
`aspire-dashboard::status` rather than assuming these:

- Web UI: `http://127.0.0.1:18888/` (`dashboard_port`)
- Console iframe proxy: `http://127.0.0.1:18887/` (`proxy_port`)
- OTLP/gRPC endpoint: `http://127.0.0.1:4317` (`otlp_port`)
- OTLP/HTTP endpoint: `http://127.0.0.1:4318` (`otlp_http_port`)
- OTLP auth: unsecured by default (`secure_otlp: false`)

`bind_host` accepts only a loopback address (`127.x.x.x`, `::1`, or
`localhost`). The dashboard process runs with `--allow-anonymous`, and OTLP
ingestion is unauthenticated whenever `secure_otlp` is false, so a
network-reachable bind would publish an open dashboard and an open trace sink.
View it from another machine through the Console instead.

Under [`iii compose`](../harness/DEVELOPMENT.md), this worker is just another
`path://` container — compose supervises it as a plain OS process, the same
way it supervises every other worker, since compose has no separate container
or VM runtime of its own.

Open the Console page and click **Export traces and logs** to configure
`iii-observability` while preserving its local trace/log stores. The button sets the configured gRPC endpoint and the trace/log exporters. iii-observability uses the configured gRPC endpoint for traces; its log exporter sends OTLP/HTTP to the paired `otlp_http_port` endpoint (`4318` by default).

`endpoint`, `exporter`, `metrics_enabled`, and `metrics_exporter` are
restart-tier fields in `iii-observability`. The write applies at the next
engine start, so restart the engine after you click the button.

`secure_otlp` defaults to `false` because `iii-observability` cannot
authenticate to a secure OTLP endpoint: its exporter sends no gRPC metadata,
and its config schema is `additionalProperties: false`, which rejects an
`otlp_api_key` or `otlp_headers` field. The only channel that reaches the
exporter is `OTEL_EXPORTER_OTLP_HEADERS=x-otlp-api-key=<key>` on the engine
process.

If you turn `secure_otlp` on, set `otlp_api_key` to an explicit value and give
the engine header the same value before you restart. Leaving `otlp_api_key`
unset makes the worker generate a key per process, which no engine header can
match, so telemetry export fails on mismatched credentials.

The Aspire Dashboard response blocks cross-origin iframes with CSP and `X-Frame-Options`. The worker's Console page therefore embeds the local proxy port, which forwards the dashboard and strips those frame-blocking headers.

## Live updates

The Console page never polls. It reads `aspire-dashboard::status` once, then
re-reads only when `aspire-dashboard::changed` fires. Bind that trigger type
with an empty config; the payload carries `reason` (`dashboard` or
`observability`) and the same dashboard snapshot `aspire-dashboard::status`
reports.

The worker emits it on dashboard process transitions, on its own configuration
changes, and on `iii-observability` configuration changes. It holds one
process-lifetime `configuration` trigger on `iii-observability` and relays
those events, so no page has to hold one itself — a `configuration` trigger
participates in the entry's TTL refcount, which is no place for a binding that
comes and goes with a browser tab.

`dashboard` events are deduplicated against the last snapshot sent, because
the lifecycle transitions overlap. A page also re-reads when the engine
connection returns and when the tab becomes visible again, which are the two
moments it can have missed an event.

`dashboard_healthy` is probed when `aspire-dashboard::status` runs, so a
dashboard that wedges without exiting is reported at the next read rather than
the moment it wedges. The process exit event covers a dashboard that dies.

## Functions

| Function | Description |
|---|---|
| `aspire-dashboard::start` | Start or reuse the standalone Aspire Dashboard process. |
| `aspire-dashboard::stop` | Stop the managed dashboard process. |
| `aspire-dashboard::status` | Report dashboard health and iii-observability export status. |
| `aspire-dashboard::configure-observability` | Update `iii-observability` to export to this dashboard. |

| Trigger type | Description |
|---|---|
| `aspire-dashboard::changed` | Fires on dashboard process transitions, on this worker's own configuration updates, and on `iii-observability` configuration updates. Empty config. |

Manual configuration equivalent for traces and logs, preserving the rest of the
current `iii-observability` value. Substitute the worker's live `bind_host` and
`otlp_port` for the default endpoint below:

```bash
iii trigger configuration::set --json "$(iii trigger configuration::get id=iii-observability \
  | jq --arg endpoint 'http://127.0.0.1:4317' \
    '.value
     | .enabled=true
     | .endpoint=$endpoint
     | .exporter="both"
     | .logs_enabled=true
     | .logs_exporter="both"
     | {id:"iii-observability", value:.}')"
```
