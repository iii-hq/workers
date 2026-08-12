# reflex

A local tool router for the harness loop. A 14MB on-device model indexes the
live engine function catalog and proposes the next function call for a
natural-language objective, with a calibrated confidence score per proposal.
It proposes only: every call it suggests still flows through the normal iii
policy, approval, and dispatch path, so adding it changes no security
boundary. It also runs a shadow mode that observes real harness turns and
builds the calibration data that tells you which confidence threshold to
trust on your own traffic.

## Install

```bash
iii worker add reflex
```

## Quickstart

Ask for the next call for an objective:

```bash
iii trigger reflex::route 'objective=list all connected workers'
```

```json
{
  "type": "call",
  "calls": [{ "function": "worker::list", "payload": {} }],
  "confidence": 0.86,
  "reasoning": "...",
  "latency_ms": 618.6
}
```

Act on proposals at or above your confidence threshold and escalate the rest
to a frontier model. To continue a chain, execute the proposed call yourself
and pass its result back as `observation`:

```bash
iii trigger reflex::route 'objective=how many chat sessions exist?' 'observation={"session_count": 50}'
```

`type` is `call`, `respond`, `abstain`, or `refuse`; `abstain` and `refuse`
mean escalate. `reflex::index::status` reports index size and freshness, and
`reflex::index::refresh` rebuilds it on demand; the index also follows the
engine automatically via the `engine::functions-available` trigger
(debounced, schema-fingerprinted, persisted across boots).

## Shadow mode

On by default. The worker binds the harness `pre-generate` and
`post-generate` hooks fail-open and observe-only: for every real generation
it predicts the next call in the background and records the frontier model's
actual calls (unwrapping `agent_trigger`, feeding prior function results
back as `observation`). Nothing about the turn changes, and a dead router
never blocks generation.

Rows land in a local jsonl that doubles as fine-tune data. The calibration
report answers "at what confidence is this router right on my rig":

```bash
iii trigger reflex::shadow::report
```

It returns turn-level proposal-vs-actual agreement per confidence bucket,
plus how many frontier generations went to catalog discovery
(`engine::functions::list/info`) — the steps a local router skips. Derive
your acting threshold from this report rather than reusing anyone else's
number; on the rig this worker was built against, proposals at confidence
0.6 and above matched the frontier's actual call in every observed case.

## Configuration

`config.yaml` next to the worker (all keys optional):

```yaml
engine_url: ws://127.0.0.1:49134   # env III_URL overrides
index_path: .index/functions.idx   # env REFLEX_INDEX_PATH overrides
shadow_log: shadow.jsonl           # env REFLEX_SHADOW_LOG overrides
shadow:
  enabled: true                    # false = router only, no hook bindings
  priority: 100                    # run after other pre-generate hooks
  timeout_ms: 2000                 # hook budget; prediction runs in background
refresh_debounce_s: 5              # coalesce catalog-change bursts
```

The shadow hooks always bind `on_error: fail_open`: a router outage must
never deny a turn.
