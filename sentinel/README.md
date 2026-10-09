# sentinel

Error monitoring for an iii engine. The engine already exports every worker's
spans and logs; `sentinel` subscribes to the failures among them, groups them
by a deterministic fingerprint so a thousand occurrences read as one problem,
and — this is the part a dashboard cannot do — **freezes the evidence at the
moment of capture**, because the engine's span store is a ring of 10 000 spans
and the trace you want to look at is usually gone by the time you click. From
a group you open an assisted harness investigation beside the page: you watch
it read the code, you steer it, and it records a structured diagnosis against
the group. Resolving and ignoring stay human decisions; regressions are
detected on their own.

## Install

```bash
iii trigger compose::add worker=sentinel worker=database worker=queue
```

`sentinel` keeps its groups in `database` (SQLite by default, zero-config) and
its ingest in a durable `queue`, so both belong in the same call: without them
the worker starts, reports itself disabled, and waits.

## Quickstart

`sentinel::status` is the one call worth knowing first. It answers whether the
monitor is actually working — not just running:

```rust
use iii_sdk::{register_worker, InitOptions};
use iii_sdk::protocol::TriggerRequest;
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let iii = register_worker("ws://localhost:49134", InitOptions::default());

    let status = iii
        .trigger(TriggerRequest {
            function_id: "sentinel::status".into(),
            payload: json!({}),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await?;

    println!("{status:#?}");
    Ok(())
}
```

```json
{
  "enabled": true,
  "engine": { "trace_store": "memory", "logs": true },
  "sources": { "trace": true, "log": true },
  "ingest": { "processed_total": 0, "redactions": 0, "lost_before_capture": 0, "…": 0 },
  "groups": { "open": 0, "regressed": 0, "ignored": 0, "resolved": 0 },
  "repositories": [{ "id": "workers", "path": "/home/me/workspaces/workers", "exists": true, "workers": ["harness"] }]
}
```

Three fields carry most of the diagnostic weight:

- `enabled` is false while the store or the queue has not registered yet —
  the worker is up, ingest is closed, and nothing is being dropped silently.
- `lost_before_capture` counts traces that left the engine's ring before the
  job ran. A number that climbs is the signal to raise the engine's
  `memory_max_spans`, not a bug in a group.
- `repositories[].exists` says whether a mapped checkout is really on this
  machine. False disables code access for its workers; grouping, evidence and
  regression keep working.

## The page

The console page appears under `errors` while the worker is up: the open
groups ordered with regressions first, and behind each one the frozen
evidence — the span that failed, the exception it carried, the tree around it
and the logs of that trace.

**Investigate** opens a harness session beside the page. You watch the agent
read the checkout mapped to the failing worker, and you can write to it at any
time; a message lands in the turn that is already running. When it has a
cause it records one by calling `sentinel::diagnosis::record`, which is the
only write to this worker its policy allows. Besides reads of the code and
the evidence, it can reach GitHub and the web, writes included (a PR merge,
a POST); the engine's raw telemetry is not on the list at all. Each recording is a version;
the most recent one stands and the earlier ones stay, so the same failure
diagnosed twice can be compared.

**Open in chat** does the same thing without running anything: the session is
created with the evidence already in the transcript and waits for you to
speak.

Each group is also **triaged** once, five minutes after it is first seen,
into `defect`, `caller_error`, `transient`, `environment` or `test_traffic`,
and the label rides on `sentinel::groups::list` and `::get` as `triage`. A
"function not found" whose function is registered by then is decided
without a model — a restart when it was brief, the environment when it was
not; everything else goes to [`judge`](../judge/) in batches. The judge sees
the group as it was stored, so already redacted. Without `judge` deployed the
groups simply stay untriaged, and a failing judge is left alone for five
minutes. A label is a hint for ordering and filtering, never a state change.
Switch it off, or change the wait, under **Triage** in the page's settings;
which judge answers is chosen in the `judge` worker's own settings.

The list opens on **Relevant**: defects, regressions, groups not triaged yet,
and caller errors or environment problems that repeat (20 or more
occurrences across an hour or more), since a program repeating a failing call
needs a fix even when its message is a polite refusal. **Noise** holds the
rest, ordered by kind, and each side shows its count, so nothing is more than
a click away. `sentinel::groups::list` takes the same choice as `relevance`.

Resolving and ignoring are yours. An ignore can last forever, for a number of
further occurrences, or until the worker version changes — and the counters
keep running either way, so an ignored group still tells you how often it
happened.

## Investigating from outside the console

```bash
iii trigger sentinel::investigate group_id=grp_... mode=assisted
iii trigger sentinel::investigations::get investigation_id=inv_...
```

The session id it answers with is a real console conversation: opening it in
the console puts you in the middle of the investigation, with the same
controls as any other chat.

## Configuration

Registered with the `configuration` worker under the id `sentinel` and
hot-reloaded in place — every key except `database` takes effect without a
restart. Each field ships with a default, so configure only what you mean to
change:

```yaml
enabled: true
sources:
  trace: { enabled: true }              # spans with status error
  log:   { enabled: true, join_window_ms: 2000 }   # an ERROR log waits this long for the span of its own trace
redaction:
  patterns: []                          # extra value shapes to redact on capture; the built-in set always runs
evidence:
  max_bytes: 1048576                    # ceiling for one frozen bundle
  settle_delay_ms: 5000                 # re-read the trace once, to catch ancestors still open at capture
retention:
  evidence_per_group: 5                 # bundles kept per group, plus the first and one per worker version
  occurrences_per_group: 1000
  resolved_ttl_days: 90
investigation:
  model: ""                             # catalog id an investigation opens with; each run may pick another
triage:
  enabled: true
  delay_ms: 300000                      # wait this long after first seen; a restart registers what it was missing
projects:                               # where a worker's source lives on this machine (formerly `repositories`, still read)
  - id: workers
    path: /home/me/workspaces/workers
    workers: [harness, ade, queue]
service_aliases: {}                     # span service.name → registered worker name, when an SDK reports a binary name
database: primary
```

There are deliberately **no turn, token or cost ceilings**: an investigation
runs beside the group's page where you can watch it and stop it, and a ceiling
that fires mid-investigation throws away the context that was about to pay
off.

Secrets never reach a model or the store: every captured value — messages,
stack traces, log bodies, attributes — passes a redactor before the first
insert, and the agent reaches live telemetry only through proxies that apply
the same redactor. `status.ingest.redactions` shows it working.

Every field and its default lives in
[`src/config.rs`](src/config.rs).
