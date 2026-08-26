# Worker-owned Console UI and activity

Workers are independent iii applications. Each worker owns its functions,
triggers, resources, and domain semantics. A worker may also ship injectable
Console UI. When it does, that UI remains part of the worker rather than
domain code embedded in Console.

Console loads and composes worker assets and provides shared presentation
primitives. Worker-provided pages, renderers, and activity views must keep
their domain semantics in the worker rather than embedding them in Console.
Console-owned chat and session orchestration may use thin worker clients when
it needs to coordinate cross-worker product behavior.

This boundary lets workers operate independently on the same iii engine while
their optional Console surfaces still feel coherent.

## State and activity

State and activity answer different questions:

- **State:** what exists or is actionable now.
- **Activity:** what happened during a known operation, session, or turn.

When a worker exposes both, current state is normally the baseline and
activity is an audit or filter view. An empty activity filter must not make
relevant current state appear absent.

Shell is the first continuing reference implementation of this distinction:

- **Uncommitted** is the default because it represents the repository's
  current working state.
- New Harness turns auto-follow **Last Turn** until the user selects another
  scope.
- A turn that finishes without file changes falls back to **Uncommitted**.
- Manually selected scopes remain stable across later turns.

These rules are not automatically applicable to every worker. A worker should
add an activity view only when its domain has a useful, truthfully correlated
activity concept.

## Ownership boundary

Every worker is responsible for its domain API and events. A worker that ships
Console UI is additionally responsible for:

1. Mapping its resources and events into domain-specific labels and actions.
2. Owning its page, renderers, filters, empty states, and resource links.
3. Working without Console-specific domain code or another optional worker.
4. Redacting secrets before data reaches its UI or activity history.

The shared `@iii-dev/console-ui` package may provide presentation primitives
such as selectors, status labels, empty states, and resource links. Those
components receive already interpreted data. They do not fetch worker
resources or assign domain meaning.

Console remains responsible for loading, composing, and disposing worker
assets, preserving portal scope, and supplying shared navigation and UI
primitives.

## Repository shape

The repository currently contains 66 top-level worker manifests. UI is
optional:

- 25 workers contain a worker-owned `ui/` asset and `src/ui.rs` registration.
- 19 of those register a full Console page.
- 6 provide injection-only renderers without a full page.
- 41 contain no worker UI.

That split is intentional. Worker independence does not require every worker
to expose a page.

## Correlation requirement

An event timestamp is not enough to label activity as "Last Turn". Multiple
Harness sessions and turns may run concurrently. Turn-scoped activity requires
correlation at the worker event boundary, such as:

- Harness session id
- turn id
- trace id
- event time
- stable worker resource id

Correlation remains optional for worker independence. Without it, a worker may
show current state and its own unscoped history, but it must not infer turn
ownership from timing.

Browser and iii-directory already own useful domain events and current-state
APIs, but their emitted events do not yet carry Harness-turn correlation. They
should not present those events as Last Turn until that correlation exists.

## Shared activity components

Shell should keep its activity UI local while it is the only non-deprecated
worker proving the pattern. Extract a shared state/activity component only
after a second worker has a real correlated use case and both implementations
demonstrate the same presentation contract.

Do not build new shared activity behavior in Editor. Editor is transitional
and is planned for deprecation after its remaining capabilities move into
Shell. New platform patterns must be proven by Shell and another continuing
worker.

## Possible future aggregation

A cross-worker activity projection or independently deployable Changes worker
may become useful later. It is not a current dependency or delivery target.
Such an aggregator would consume explicit worker-owned activity records and
link back to worker-owned resource views. It must not infer domain changes by
polling APIs or move worker semantics into Console.

## Delivery order

1. Complete and verify Shell's current-state and Last Turn behavior.
2. Move remaining Editor capabilities into Shell without using Editor as a
   second activity implementation.
3. Add correlation to another worker only when its product flow needs a
   truthful session or turn view.
4. Implement that activity view inside the worker.
5. Extract repeated presentation pieces into `@iii-dev/console-ui` only after
   both implementations prove the API.
6. Re-evaluate cross-worker aggregation from demonstrated needs.

## Non-goals

- Requiring every worker to ship Console UI.
- A Console-owned registry of worker resource semantics.
- Guessing turn ownership from timestamps.
- Treating a successful function call as proof that an agent turn succeeded.
- Depending on Editor as a new shared-platform reference.
- Replacing worker-specific history and diagnostics with a generic feed.
