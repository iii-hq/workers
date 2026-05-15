# harness

Meta-worker that composes fifteen modular workers into a runnable iii chat surface, exposes a browser-facing HTTP bridge (`bridge::trigger`, `bridge::events`), and ships a Vite/React UI that talks to the bus through it. The harness does not own chat, agent, or provider logic; it registers a small set of bus functions and expects peers such as `turn-orchestrator`, `provider-router`, shell tools, and related workers to be installed alongside it. Deeper layout and streams behavior are documented in [`ARCHITECTURE.md`](ARCHITECTURE.md).

## Install

```bash
iii worker add harness
```

`iii worker add` fetches the binary, writes a config block into `~/.iii/config.yaml`, and the engine starts the worker on the next `iii start`.

To register the harness skill bundle metadata with the bus (the worker does this automatically at boot when `skills` is available), ensure the [skills](../skills) worker is part of your stack:

```bash
iii worker add skills
```

## Quickstart

After `iii start`, probe the bundle and list expected runtime workers:

```rust
use iii_sdk::{register_worker, InitOptions, TriggerRequest};
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let iii = register_worker("ws://127.0.0.1:49134", InitOptions::default());

    let result = iii
        .trigger(TriggerRequest {
            function_id: "harness::status".into(),
            payload: json!({}),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await?;

    println!("{result:#?}");
    Ok(())
}
```

Forward an arbitrary bus call through the HTTP-oriented bridge (same shape as `bridge::trigger` on the engine):

```rust
// function_id / payload match iii.trigger(...)
let result = iii
    .trigger(TriggerRequest {
        function_id: "bridge::trigger".into(),
        payload: json!({
            "function_id": "models::list",
            "payload": {},
        }),
        action: None,
        timeout_ms: Some(240_000),
    })
    .await?;
```

Registered functions (use `::` ids on the bus):

| Function | Role |
|---|---|
| `harness::status` | Bundle name, version, and expected worker list (cheap liveness probe). |
| `bridge::trigger` | Forwards `{ function_id, payload }` to `iii.trigger`. HTTP: `POST` `bridge/trigger`. |
| `bridge::events` | SSE-style tail of `agent::events` for a session. HTTP: `GET` `bridge/events`. |

`bridge::trigger` is not meant as an LLM tool — it is the browser’s call-anything escape hatch.

## Configuration

```yaml
# Default engine WebSocket URL when III_URL / --url are unset
engine_url: "ws://127.0.0.1:49134"
```

Other runtime flags:

- `--config` — path to this file (default `./config.yaml`; override with `III_HARNESS_CONFIG`).
- `--url` or `III_URL` — engine WebSocket URL; wins over `engine_url` in the file.

Registry-facing defaults also appear in `iii-harness --manifest` under `default_config`.

## Expected workers

`EXPECTED_WORKERS` (in [`src/lib.rs`](src/lib.rs)) is generated at build time
from the `dependencies:` block of [`iii.worker.yaml`](iii.worker.yaml) by
[`build.rs`](build.rs). Add or remove a worker by editing `iii.worker.yaml`
only — the Rust constant rebuilds automatically.

## Trace correlation

Every harness-registered function wraps its body in an OTel span tagged with
`iii.session.id`, `iii.message.id`, and (for `bridge::trigger` only)
`iii.function.id`. The HTTP response carries two new headers when
observability is active:

- `traceparent: 00-<trace_id>-<span_id>-01` — W3C trace context for the span
  that wrapped this call.
- `x-iii-message-id: <id>` — the `message_id` you sent on the request, or
  the upstream value propagated via OTel baggage. **Omitted entirely** when
  neither source supplied one. This keeps plumbing calls (UI subscribes,
  status polls, engine-internal traffic) out of `Group by message` in the
  console — only real chat-turn IDs land there.

Discover harness traces in the iii Developer Console TRACES tab, or via:

```bash
# By span name (any harness function):
iii trigger --function-id engine::traces::list \
  --payload '{"name":"harness.status","search_all_spans":true}'

# By message_id directly (engine v0.11.7+ — needs the search_all_spans
# attribute-filter widening + iii-sdk BaggageSpanProcessor; works on
# every span in the trace, not just the harness-wrapped one):
iii trigger --function-id engine::traces::list \
  --payload '{"attributes":[["iii.message.id","<msg-id>"]],"search_all_spans":true}'

# Server-side aggregation (engine v0.11.7+):
iii trigger --function-id engine::traces::group_by \
  --payload '{"attribute":"iii.message.id"}'
```

Both headers are absent when the iii-observability worker is not running
(see `harness/config.yaml`). Web clients should treat them as optional —
"`traceparent` absent" means "observability is off," not "the call failed."

#### Operator observability of the wrapper itself

The wrapper emits a `tracing::trace!` event per span entry with `fn_name`,
`recording` (whether OTel is active), `session_id`, and `message_id_minted`
(always `false` since the harness no longer mints; kept for log-format
stability).
Tail with `RUST_LOG=harness::otel=trace` to detect the
"observability worker went silent" failure mode (rising `recording=false`
rate without an OTel runtime change).

#### Baggage propagation (and why TRACES doesn't group by message_id yet)

The wrapper also writes `iii.session.id`, `iii.message.id`, and (for
`bridge::trigger` only) `iii.function.id` into the OTel **baggage** of the
context attached around the handler. Every downstream `iii.trigger(...)`
call ships the baggage on the wire automatically (iii-sdk's `inject_baggage`
is wired into the invocation message at `iii-sdk/src/iii.rs:312`). Receiving
workers extract it via `extract_context(traceparent, baggage)` and the
entries live in their task-local OTel context for the duration of the
handler.

What this does NOT do yet: **baggage entries are not automatically copied
onto span attributes** of downstream worker spans. The OTel SDK requires an
explicit `SpanProcessor` that reads baggage on `on_start` and writes it to
the span as attributes; none exists in `iii-observability` today. So in the
iii Developer Console TRACES tab, downstream spans (e.g. `state::set`,
`approval::list_pending`) still appear without `iii.message.id` even though
the baggage *is* travelling alongside them.

Required engine-side follow-up to make TRACES group by message:

1. Add a span processor in `iii-observability` that copies a configurable
   allowlist of baggage keys onto each span at start time (allowlist defaults
   to `iii.session.id`, `iii.message.id`, `iii.function.id`).
2. Optionally extend `engine::traces::list` so `search_all_spans: true` also
   applies the attribute filter (currently root-only — documented above).
3. Optionally, surface a "group by attribute" affordance in the TRACES tab.

Once (1) lands, every span in the trace inherits the ids automatically. The
harness side is forward-compatible: the baggage is already flowing.

### Direct-bus return shapes

Two functions return the HTTP-trigger envelope `{status_code, headers, body}`:
`bridge::trigger` and `bridge::events`. The other five — `harness::status`,
`bridge::info`, `ui::subscribe`, `ui::unsubscribe`, `harness::fs::read_inline`
— return their raw payloads so direct-WebSocket callers (the web `StatusPill`,
`fetchBridgeInfo`, `ui::subscribe` registration, FilesystemPanel reads) can
read fields off the top level. The OTel span still fires with `iii.*`
attributes for all seven; only the HTTP `traceparent` / `x-iii-message-id`
header echo is skipped for the five raw-shape functions (their wrapper sees
no `status_code` in the return and leaves headers untouched).

> **Contract reminder.** Any change to a wrapped function's return shape
> (envelope ↔ raw) is a breaking change for direct-WS consumers in
> `harness/web/`. Commit `767c83d` reverted four functions from envelope back
> to raw after the unified-envelope rollout broke `StatusPill.tsx` at
> runtime. The wrapper variants document the contract at the call site —
> `with_envelope_span` for `bridge::trigger`/`bridge::events`, `with_raw_span`
> for the other five. Keep them aligned with their consumers.

### Wildcard subscriptions

`ui::subscribe` / `ui::unsubscribe` accept `session_id: null` to mean "all
sessions." In TRACES those calls show up with `iii.session.id = "*"`.

Contributor commands (fmt, clippy, tests) for this crate live in [`binary-worker.md`](../binary-worker.md) §11; source layout notes are in [`ARCHITECTURE.md`](ARCHITECTURE.md).
