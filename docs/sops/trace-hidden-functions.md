# Hiding a function's spans in traces (`trace_hidden`)

How to mark a function as **hidden by default in trace UIs**: one boolean in
its registration metadata. The spans are still recorded and stored — nothing
changes on the wire or in the engine — but the console's traces page starts
with those span groups filtered out, and users can unhide them from the span
filter (the funnel menu) whenever they want. Reference implementations:
`session-manager` (all functions), `context-manager` (all functions),
`harness` (`harness::turn` only).

Use this when a function's spans are **plumbing that repeats on every
operation** and drowns the signal:

- per-turn bookkeeping — the session manager's `session::update-message`
  fires dozens of times per agent turn;
- dispatch machinery whose spans duplicate a better span — `harness::turn`
  stacks a queue wrapper (`fn_queue`), an SDK handler span
  (`execute harness::turn`), and the producer's own `harness::turn step`
  scope span around the same turn; only the step span is worth reading (for
  sub-agents it carries the task title);
- internal protocols — `session::store::*` crosses the bus on every
  mutation in bridged deployments.

Do **not** hide functions whose spans ARE the signal: LLM calls, tool
executions, user-facing work. Hiding is a default, not access control — the
spans remain visible one funnel-click away, in `engine::traces::*` RPCs, and
in any other OTel consumer.

## 1. The contract

| Layer | Key | Meaning |
|---|---|---|
| Function registration metadata | `trace_hidden: true` | Trace UIs hide this function's span groups by default |

That is the whole contract. It rides the same free-form metadata object as
the existing `internal: true` convention (`engine::functions::list` returns
it back), so it needs no SDK or engine changes and works from every SDK
language.

**Who consumes it.** The console's traces page
(`workers/console/web/src/lib/trace-hidden-functions.ts`) fetches
`engine::functions::list { include_internal: true }`, collects the ids whose
`metadata.trace_hidden === true`, and merges them into the span filter as
defaults. A user unhiding a group persists that override in the `console`
configuration entry (`traces.spanFilters.shownGroups`), so the default never
fights an explicit choice. Other trace UIs can adopt the same rule.

## 2. Tagging a function (Rust SDK)

Chain `.metadata` onto the registration:

```rust
iii.register_function(
    "myworker::sync",
    RegisterFunction::new_async(move |req: SyncRequest| async move { /* … */ })
        .description("Internal: reconcile the mirror after every mutation.")
        .metadata(serde_json::json!({ "trace_hidden": true })),
);
```

Workers that hide **all** their functions put the tag in their shared
`register` helper instead of on each call site — see
`session-manager/src/functions/mod.rs` and
`context-manager/src/functions/mod.rs`. Workers that hide **one** function
keep a dedicated variant so the default stays visible — see
`register_trace_hidden` in `harness/src/functions/mod.rs`.

If the function already carries other metadata (e.g. `internal: true`),
merge the keys into one object: `json!({ "internal": true, "trace_hidden":
true })` — `.metadata()` replaces the whole value.

## 3. What exactly gets hidden

The console groups spans by **owning function id**; hiding a group removes
**only the group's own spans** — a hidden span's children stay visible and
re-attach to the hidden span's parent (its nearest visible ancestor), so
the hierarchy never loses work: a hide is a de-noise, not a subtree
collapse. A hidden function's own spans are:

- the engine's dispatch spans for it (`enqueue <fn> → <queue>`,
  `fn_queue <queue>` — they carry a `function_id` attribute);
- the worker SDK's handler span (`execute <fn>`);
- spans attributed to the function by baggage (`iii.function.id`) — its
  internal client calls and other in-scope work.

Chained plumbing still disappears as a unit because each link matches on
its own: hiding `harness::turn` removes `enqueue` + `fn_queue` +
`execute harness::turn` (all attributed to `harness::turn`), and the
`harness::turn step` span — a tag ROOT, so it groups under its own name
(see [`timeline-span-tags.md`](../../console/docs/timeline-span-tags.md))
rather than under `harness::turn` — re-parents under `harness::send` with
all the turn's real work intact. A span nested inside hidden plumbing that
belongs to a *different, visible* function always survives, promoted.

So a producer that wants "hide my dispatch shell, keep my meaningful span"
implements BOTH halves: `trace_hidden` on the function, and a tagged scope
span (`iii.tag.kind`) inside the handler marking the segment worth keeping
under its own name. The harness is the canonical example.

## 3a. Call-site hiding: `iii.tag.hidden` and the "internal" filter section

Function-level metadata hides a function from EVERY caller. When a function
is meaningful from some call sites and plumbing from others (`state::*`,
`session::update-message`), tag the CALL instead: wrap the outbound call in
a baggage scope carrying `iii.tag.hidden = <family>`:

```rust
iii_helpers::observability::run_with_baggage(
    &[("iii.tag.hidden", "harness state")],
    iii.trigger(TriggerRequest { function_id: "state::get".into(), /* … */ }),
)
.await
```

The baggage stamps every span the call produces — the engine `call <fn>`
span, the callee's `execute <fn>` span, descendants — so each span of the
delivery hides on its own match (an untagged descendant, e.g. from a worker
whose SDK drops the baggage, stays visible and re-parents like any other
child of a hidden span). The console's span filter shows tagged spans in a
separate **internal** section (not under the `iii` worker entry), grouped
by the family label, hidden by default; unhiding a family persists as
`traces.spanFilters.shownInternal`. Families in use: `harness state`
(harness `src/state.rs`), `session updates` (harness streaming
`session::update-message` writes), `session events` (session-manager's
event fan-out — the console's live relays), `turn enqueue` (harness
re-enqueue of the next `harness::turn` step). See
[`timeline-span-tags.md`](../../console/docs/timeline-span-tags.md).

## 3b. Engine built-ins

Built-in functions (`configuration::*`, `state::*`, `engine::*`, …) execute
in-process in the engine — there is no worker, so the engine's own
`call <fn>` span is the invocation's only possible record. The engine emits
it when the call arrives **with caller trace context** (a `traceparent`):
an agent turn calling `configuration::list` shows that call nested in the
turn, with error status and an `exception` event when it fails. The span
carries `iii.function.kind: internal` plus `function_id`, so it groups
under the function id in the span filter like any worker function.

Three deliberate boundaries:

- **Never top-level.** Context-free built-in calls (console RPC polling,
  boot-time reads, the engine's own machinery) emit no span at all, so
  built-ins never root new rows in the trace list. The list additionally
  filters internal root spans as a second guard.
  `III_OTEL_TRACE_BUILTINS=true` (engine env) forces spans for context-free
  calls too, for debugging.
- **Observability functions are NEVER traced** — `engine::traces::*`,
  `engine::logs::*`, `engine::log::*`, `engine::metrics::*`, and the rest
  of the observability worker's surface
  (`telemetry::is_observability_function_id`), with or without caller
  context, even under the env override. The pipeline must not observe
  itself: a traced delivery of the devtools span feeds re-enters the feed
  it delivers and loops endlessly. For the same reason the engine's
  `iii:devtools:*` stream fan-out runs without spans or trace context
  (`stream.rs::invoke_triggers`), and the live-feed subscriber drops any
  observability-attributed span as a belt.
- **Hidden only when configured.** Built-in spans inside a trace are shown
  by default; hide a family from the funnel (persisted), tag its
  registration `trace_hidden: true`, or tag noisy CALL SITES with
  `iii.tag.hidden` (§ 3a).

Policy lives in `telemetry::should_suppress_invocation_span`
(`iii/engine/src/workers/telemetry/mod.rs`); the live-feed loop-break that
keeps the engine's own machinery spans out of the console streams is
`is_context_free_internal_span`
(`iii/engine/src/workers/observability/mod.rs`).

## 4. Checklist

- [ ] The function's spans are noise (per-operation plumbing), not signal.
- [ ] `.metadata(json!({ "trace_hidden": true }))` chained on the
      registration (or the worker's shared `register` helper, when all of
      its functions qualify).
- [ ] Existing metadata keys preserved — `.metadata()` replaces the object.
- [ ] If the handler does work a human should still see, that work is
      marked as its own segment via `iii.tag.kind`
      ([`timeline-span-tags.md`](../../console/docs/timeline-span-tags.md))
      so the hide re-roots it instead of swallowing it.
- [ ] Verify the registration: restart the worker, then
      `iii trigger engine::functions::list --json '{"include_internal": true}'`
      and check the function's `metadata.trace_hidden`.
- [ ] Verify the console: open Traces — the group starts hidden (funnel
      badge counts it); toggling it back on works and survives a reload.

## 5. Troubleshooting

- **Group still visible after deploy** — the console caches the function
  list briefly (React Query, 5 min) and the user may have unhidden the
  group earlier: an entry in `traces.spanFilters.shownGroups` of the
  `console` configuration entry overrides the default forever. Clear it
  from the funnel (re-hide) or edit the config entry.
- **Everything under the function vanished, including a span you wanted** —
  the kept span must be a tag ROOT: it needs an `iii.tag.kind` value its
  ancestry does not already carry (baggage smear makes descendants echo
  the kind; echoes do not start segments). Root-ness compares against the
  nearest tagged ancestor, so tag-less gap spans from older-SDK workers
  neither hide a scope from its descendants nor promote echoes.
- **The group never appears in the funnel at all** — entries derive from
  spans present in the current window; a function that emitted nothing has
  nothing to list.
