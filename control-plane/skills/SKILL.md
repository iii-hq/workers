---
name: control-plane
description: >-
  Read a project's live management view, decisions, runs, pull requests, and
  referenced summary without acting on a person's behalf.
---

# control-plane

Control plane combines harness, kanban, session and optional GitHub facts into a
read-only project view. Use it when an agent needs the same scoped portrait as
the console without reproducing the aggregation and urgency rules.

The summary writer is optional and runs only when a model is configured. Pull
request data is unavailable when the GitHub worker is absent.

## When to Use

- Read the management-by-exception overview for one project or all projects.
- Find open decisions, assumptions, stalled roots or active runs in control plane order.
- Observe scoped changes instead of polling every underlying worker.

## Boundaries

- Control plane reads facts and refreshes its own summary.
- Use harness to resume runs, harness or kanban to answer questions, and GitHub
  for pull request actions.

## Functions

- `control-plane::overview` — read the Overview facts, live work and summary freshness for a scope.
- `control-plane::needs-you::list` — list urgent and non-urgent questions plus stalled roots.
- `control-plane::runs::list` — list scoped root runs with state and context size.
- `control-plane::prs::list` — list cached pull requests from configured project repositories.
- `control-plane::decided::list` — page through settled project decisions.
- `control-plane::assumptions::list` — list recorded assumptions, optionally since a timestamp.
- `control-plane::watch` — keep a scope actively refreshed for two minutes.
- `control-plane::summary::refresh` — request one rate-limited summary writer turn.
- `control-plane::on-changed` — emit the scope, timestamp and read-model parts that changed.

## Observing events

Observe `control-plane::on-changed` when another function should refetch only the parts
of a scope whose read model changed. It avoids polling all source workers.

Register the target function, then observe the event for the desired scope:

```typescript
iii.observe(
  {
    function_id: 'control-plane::on-changed',
    filters: { 'input.scope': { $eq: 'all' } },
    capture: ['output'],
  },
  'consumer::on-control-plane-changed',
)
```

Call `control-plane::watch` while a view is open; observing the change event alone does
not establish a repository watch lease. See the API reference for the event
payload and schemas.
