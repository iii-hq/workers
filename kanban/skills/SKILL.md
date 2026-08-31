---
name: kanban
description: >-
  Plan, launch, inspect, review, and land multiple Harness or external-worker
  tasks as one repository-aware run with dependencies and isolated worktrees.
---

# kanban

Kanban is a visual control plane over the live Harness session tree. Harness
and the session manager own execution, messages, status, and parent-child
relationships. The worktree Worker owns isolated Git directories, branches,
status, and landing. Kanban binds them through durable run definitions,
executor selection, dependencies, retries, and explicit review acceptance.

## When to use

- A task has independent workstreams that should execute concurrently.
- A person wants to see existing Harness child sessions as cards and open the
  underlying conversation from the board.
- External workers exposing the shared task contract should participate in the
  same run as native Harness sessions.
- Parallel implementation tasks need separate branches and filesystem roots.
- A dependent task should wait until its prerequisite reaches Review.
- Completed worker output needs an explicit Review to Done gate.

## Workflow

1. Call `kanban::executors::list` and select only executors whose `available`
   field is true.
2. Create one coherent run with `kanban::runs::create`. Set `repo_path` and
   `isolation: worktree_per_task` for parallel Git work. Give every task a
   stable `key`, bounded instruction, executor ID, and descriptive title. Use
   `depends_on` keys for ordering and `auto_dispatch: false` only when the
   person wants a staging gate.
3. Read `kanban::board` after `kanban::changed` events. Do not poll it on a
   timer.
4. Open `root_session_id` or a task's `external_session_id` when the detailed
   transcript is needed.
5. Retry only `needs_you` tasks. Accept only tasks in `review` after checking
   their result.
6. Call `kanban::tasks::land` only after review and only with the intended
   target branch. Landing is an explicit operator action, never automatic.

## Boundaries

- Do not create a second message bus or scheduler. Use the existing session,
  state, and Harness functions.
- Do not invent an executor from its name. Executor availability comes from
  the live function registry.
- Do not create ad-hoc directories for isolated work. Use `worktree::create`
  through the run isolation contract and pass its path to the child executor.
- Harness-projected runs are read-only board projections. Their session tree
  remains authoritative.
- Kanban-owned task controls must use the `kanban::tasks::*` functions so
  durable metadata and reactive updates remain consistent.

## Functions

| Function | Use |
|---|---|
| `kanban::board` | Read live runs, task routes, worktrees, Git status, and capabilities. |
| `kanban::executors::list` | Discover usable executors and capabilities. |
| `kanban::runs::create` | Create a native root session and durable tasks. |
| `kanban::tasks::dispatch` | Start a held task. |
| `kanban::tasks::retry` | Re-run an attention-blocked task. |
| `kanban::tasks::stop` | Stop a task when supported. |
| `kanban::tasks::accept` | Approve a reviewed result. |
| `kanban::tasks::land` | Queue reviewed Git work onto the run target branch. |

The Console page ID is `kanban`. Open it from another injected page with
`host.panels.open({ pageId: 'kanban' })`.
