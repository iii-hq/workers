# Timeline span tags: `iii.tag.kind` / `iii.tag.display_name`

A convention for marking OpenTelemetry spans as **relevant** — the spans a
trace UI should treat as first-class segments (an agent turn, a sub-agent
run, a queue-dispatched job) rather than anonymous `execute <fn>` bars. Two
span attributes carry the whole contract:

| Attribute | Value | Meaning |
|---|---|---|
| `iii.tag.kind` | free-form string, dot-namespaced (`harness.turn`, `queue.process`, …) | classifies the span; its **presence** is what makes a span a relevant-span candidate |
| `iii.tag.display_name` | free-form human string | overrides the span's display label wherever it renders |

Nothing here deviates from OTel: producers stamp plain span attributes,
either directly or via W3C baggage, and consumers read them back off the
stored span. No custom protocol, no new wire format. The convention is
UI-vendor-neutral — any timeline that renders spans can consume it.

**Registered kinds.** The value space is open; producers namespace by
domain. Kinds in use today:

| `iii.tag.kind` | Producer | Display-name convention |
|---|---|---|
| `queue.process` | a queue worker's dispatch wrapper — THE "function trigger" span of one queue delivery | `<function> (<queue>)`, e.g. `harness::turn (default)` |
| `harness.turn` | an agent harness's top-level turn step | none (the default label is already right) |
| `harness.subagent` | a turn step belonging to a spawned sub-agent | `Sub-agent · <task preview>` |
| `harness.spawn` | the act of spawning a sub-agent | `Spawn · <task preview>` |

A display name is only worth setting when it is genuinely more informative
than the default verb-stripped span name — `Workflow: cleanup temp files`
beats `execute workflow::step`; a display name that just repeats the
function name is noise.

---

## 1. How tags get onto spans

Two mechanisms, chosen by span topology:

**Direct span attributes** — when the producer owns the exact span and wants
the tag on that span only. The queue dispatch wrapper is the canonical case:
the consumer loop opens one span per delivery and stamps it at creation.
Nothing leaks to child spans.

```rust
// Sketch: the queue worker's per-delivery span.
let span = tracing::info_span!(
    "fn_queue_job",
    otel.name = %format!("fn_queue {queue_name}"),
    function_id = %function_id,
    "messaging.destination.name" = %queue_name,
    "iii.tag.kind" = "queue.process",
    "iii.tag.display_name" = %format!("{function_id} ({queue_name})"),
);
```

**Baggage scope** — when the producer's work spans many spans (its own and
downstream services'), set the tags as W3C baggage around the work. A
baggage-aware span processor (one that copies baggage entries onto every
span **started inside the scope**) materializes them as attributes; baggage
also propagates across process boundaries, so downstream spans inherit the
tags. The agent-harness turn is the canonical case:

```rust
// Sketch: an agent turn step. `run_with_baggage` sets the baggage scope,
// and the explicit inner span guarantees at least one span STARTS inside
// it (baggage never lands on spans that were already open).
let baggage = [
    ("iii.tag.kind", if is_subagent { "harness.subagent" } else { "harness.turn" }),
    // sub-agents also push ("iii.tag.display_name", "Sub-agent · <task preview>")
];
run_with_baggage(&baggage, async {
    run_in_span("harness::turn step", None, || run_step(payload)).await
})
.await
```

### The smear, and tag scope ROOTS

Baggage is inherited: every span started inside the scope — the turn's LLM
calls, tool calls, state reads, and anything they call — repeats the same
`iii.tag.*` attributes. That is fine (it's what makes trace-level tag
merging work) but it means *"has `iii.tag.kind`"* does **not** identify the
interesting span.

The convention's identity rule: the relevant span of a scope is its **tag
root** — a span whose parent does *not* carry the same `iii.tag.kind` value
(or that has no stored parent). Descendants repeating the value are echoes;
a **changed** value nested inside another scope (a sub-agent turn inside a
parent turn, a spawn inside a step) starts a new scope and is a fresh tag
root. Backends and consumers must apply this rule wherever they enumerate
relevant spans; a producer stamping direct attributes on one span trivially
satisfies it.

**Queue boundaries reset the scope.** A publisher's baggage necessarily
carries its own scope's tags, and they ride the queued message; replayed
verbatim they would smear the *publisher's* identity over the whole
delivery subtree (and duplicate the wrapper's own direct keys). The queue
consumer therefore scrubs `iii.tag.kind` / `iii.tag.display_name` — and
only those — from the inbound baggage before attaching it: identity and
lineage keys (`iii.session.id`, `iii.tag.message`, …) flow through, the
dispatch wrapper stamps its own `queue.process` identity, and the consumed
function re-stamps its own tags.

---

## 2. Consumer: the per-trace detail waterfall

The lightest consumption — per rendered span:

- **Label**: `iii.tag.display_name`, when present, wins outright over the
  default (verb-stripped) span name.
- **Icon/classification**: `iii.tag.kind` is checked before the raw OTel
  `SpanKind` when bucketing the span's icon. `queue.process` buckets with
  queue consumers/producers; `harness.*` buckets with function invocations.

Untagged spans are untouched, so the convention is strictly additive.

---

## 3. Producer checklist

A producer is done when:

- [ ] `iii.tag.kind` lands on every span it wants classified — stamped
  directly on a span it owns, or via a baggage scope with an explicit inner
  span so the scope has a tag root.
- [ ] The tag-root rule holds: the span carrying the *new* kind value is the
  one that should render; check the parent doesn't already carry the same
  value.
- [ ] `iii.tag.display_name` is set only where it beats the default label,
  following the kind's display convention (see the registry table).
- [ ] Tags are bounded: a scope per unit of work (a turn, a delivery, a
  job), never per high-frequency inner operation — consumers treat every
  tagged scope as a first-class segment.
- [ ] The function identity is readable off the span (a `function_id`-style
  attribute or the span name), so consumers can group and select by
  function.
- [ ] Existing consumers still typecheck/build against the new values —
  kinds are open-vocabulary, so nothing should need wiring, but a typo in
  the attribute KEYS breaks silently.

**Transport requirement.** The baggage mechanism assumes the span processor
copies *all* baggage keys onto span attributes (early processors allowlisted
a fixed key set, which silently drops `iii.tag.*`). Direct span attributes
have no such dependency.

---

## 4. Cookbook: tagging a queue-triggered worker

The shape to copy for any worker whose functions run as enqueued jobs and
should read as first-class segments:

```rust
// Inside the enqueued function's handler, before doing the work:
run_with_baggage(
    &[
        ("iii.tag.kind", "workflow.step"),
        ("iii.tag.display_name", &format!("Workflow: {step_name}")),
    ],
    async {
        run_in_span("workflow::step", None, || run_step(payload)).await
    },
)
.await
```

With a queue worker that already tags its dispatch wrapper (`queue.process`
+ `fn (queue)`), this inner tagging is optional polish: the job already
shows up in timelines through the wrapper. Add the inner scope when the
producer knows something the queue does not — a suggestive per-item name, a
domain-specific kind. A consumer rendering both can fold the wrapper and
its same-function inner span into one well-labelled segment (nested
intervals, inner display name wins).
