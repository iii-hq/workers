---
name: tower
description: >-
  Read a project's live management view, decisions, runs, pull requests, and
  referenced summary without acting on a person's behalf.
---

# tower

Tower combines harness, kanban, session and optional GitHub facts into a
read-only project view. Use it when an agent needs the same scoped portrait as
the console without reproducing the aggregation and urgency rules.

The summary writer is optional and runs only when a model is configured. Pull
request data is unavailable when the GitHub worker is absent.

## When to Use

- Read the management-by-exception overview for one project or all projects.
- Find open decisions, assumptions, stalled roots or active runs in tower order.
- Observe scoped changes instead of polling every underlying worker.

## Boundaries

- Tower reads facts and refreshes its own summary; it never answers questions,
  resolves approvals or resumes runs.
- Use harness, kanban, approval-gate or GitHub directly for actions.
- Held approval calls are read by the console and are absent from tower data.

## Functions

- `tower::overview` — read direction, exceptions, live work and summary freshness for a scope.
- `tower::needs-you::list` — list urgent and non-urgent questions plus stalled roots.
- `tower::runs::list` — list scoped root runs with state and context size.
- `tower::prs::list` — list cached pull requests from configured project repositories.
- `tower::decided::list` — page through settled project decisions.
- `tower::assumptions::list` — list recorded assumptions, optionally since a timestamp.
- `tower::watch` — keep a scope actively refreshed for two minutes.
- `tower::summary::refresh` — request one rate-limited summary writer turn.
- `tower::on-changed` — emit the scope, timestamp and read-model parts that changed.

## Observing events

Observe `tower::on-changed` when another function should refetch only the parts
of a scope whose read model changed. It avoids polling all source workers.

Register the target function, then observe the event for the desired scope:

```typescript
iii.observe(
  {
    function_id: 'tower::on-changed',
    filters: { 'input.scope': { $eq: 'all' } },
    capture: ['output'],
  },
  'consumer::on-tower-changed',
)
```

Call `tower::watch` while a view is open; observing the change event alone does
not establish a repository watch lease. See the API reference for the event
payload and schemas.
