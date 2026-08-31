# kanban

Repository-aware multi-worker runs in the Console. The **Kanban** page combines
a root Harness session, native or external child executors, dependency gates,
isolated Git worktrees, review, and explicit landing in one Kanban board.
Existing Harness conversations with child sessions appear automatically, while
runs created from Kanban receive a real root session so every task remains
visible in the Console's native conversation hierarchy.

Harness and the session manager remain the execution source of truth. Kanban
stores only run definitions, task dispatch metadata, dependencies, workspace
references, and review decisions. The worktree Worker owns Git lifecycle and
status. Reactive triggers keep the page current without another scheduler.

## Install

```bash
iii compose add kanban
```

The page appears in the Console as **Kanban** (`#/ext/kanban`). It discovers
available executors from the live function registry. Harness is supported
natively; workers that expose a `::task` function using the durable
`agent_tasks` result contract are added automatically.

## What the page controls

- Select any live run and open its root Harness conversation.
- Create up to 24 tasks in one run and choose an executor for each task.
- Give every task its own managed worktree and branch, or use one shared directory.
- Gate tasks on reviewed prerequisites, then dispatch them automatically.
- Mix native Harness agents with external task-contract executors under one root.
- Dispatch immediately or hold tasks in Ready.
- Open the child session or a Shell rooted in the task worktree.
- Stop supported running tasks, retry failed tasks, and accept reviewed work.
- Inspect live Git status and explicitly queue reviewed work for serialized landing.
- Observe existing parent and child Harness sessions without importing or
  duplicating them.

## Functions

| Function | Purpose |
|---|---|
| `kanban::board` | Return runs, tasks, executor topology, managed-worktree Git status, and capabilities. |
| `kanban::executors::list` | List currently available Harness and task-contract executors. |
| `kanban::runs::create` | Create a root session plus one or more durable tasks, optionally dispatching them immediately. |
| `kanban::tasks::dispatch` | Dispatch a Ready or attention-blocked task. |
| `kanban::tasks::retry` | Retry an attention-blocked task, preserving the claimed session for a managed worktree. |
| `kanban::tasks::stop` | Stop a running task when its executor exposes a stop function. |
| `kanban::tasks::accept` | Move a reviewed task to Done. |
| `kanban::tasks::land` | Queue a reviewed task's worktree for tested, serialized landing. |

## Reactive updates

Bind `kanban::changed` with an empty config and re-read `kanban::board` when it
fires. The trigger is emitted for Kanban state writes and relevant session
creation, status, and metadata events.

```ts
iii.registerTrigger({
  type: 'kanban::changed',
  function_id: 'my-worker::on-kanban-change',
  config: {},
})
```

## Run from source

```bash
pnpm install
pnpm test
pnpm typecheck
pnpm build
III_URL=ws://127.0.0.1:49134 III_NAMESPACE=my-project pnpm start
```

A standalone process must use the same namespace as the Console and Harness.
Set `III_KANBAN_UI_WATCH=1` to serve page assets from `ui/dist` while developing
the injected UI.
