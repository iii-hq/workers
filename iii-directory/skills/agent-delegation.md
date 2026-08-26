---
name: agent-delegation
type: how-to
description: >-
  Hand a whole task to another coding agent on this engine — Claude Code, pi,
  or any worker that answers the agent contract — and be WOKEN when it
  finishes. Read it when you are about to say a model or provider is
  unavailable, or when you are about to hold a call open for the length of
  someone else's agent run.
---

# Delegating a task to another agent

Another agent on this engine is **not a model and not a provider**. Asking
`router::provider::list` for `claude` or `pi` returns nothing, and that answer
is correct: those are workers, and you reach them the way you reach every
worker — by calling a function.

    iii trigger engine::functions::list --json '{"search":"task"}'

## The one call

| Function        | What it is                              |
| --------------- | --------------------------------------- |
| `claude::task`  | Delegate one task to Claude Code        |
| `pi::task`      | Delegate one task to pi                 |
| `<agent>::task` | Any agent worker that adopts this shape |

```bash
iii trigger claude::task --json '{
  "task": "Read src/auth.ts and list every caller of verifyToken",
  "parent_session_id": "<your own session id>",
  "cwd": "/path/to/repo"
}'
# → { "session_id": "…", "started": true }
```

It returns immediately. The child is a real session: `parent_session_id` links
it to yours, so the console nests it under your turn the way a sub-agent nests.

**Write the task as if the child can see nothing you can see** — it cannot. Name
the repository, the file, the database, the resource ids, in the task itself.

## Being woken, instead of waiting

Nothing new to learn here: a settled task is an ordinary `state` write, and you
bind it the way you bind any other trigger — which the calling rules already
tell you to do instead of polling.

- scope `agent_tasks`
- key the child's `session_id`
- value `{ session_id, parent_session_id, agent, task, status, result, error, updated_at_ms }`

Bind before you delegate. Take the trigger's config keys from
`engine::triggers::info id=state`, never from memory.

The child's turns also stream onto `agent::events` under its `session_id`, and
`session::get` returns its status and title. Those are for watching; the state
write is what wakes you.

## When blocking IS right

`run::start_and_wait` (and its per-worker aliases `claude::run` / `pi::run`)
runs the turn and returns the result. Use it from a script, a shell, or a tool
that has nothing else to do. Inside an agent turn, prefer `::task`: a blocking
call spends your turn budget waiting, and a restart loses the answer, while a
state write survives both.

## What each agent is for

- **`claude::task`** — Claude Code with its own tools and login.
- **`pi::task`** — the pi coding agent, provider-agnostic.

Both stream their work onto `agent::events`, so the console renders a delegated
run exactly like a native harness sub-agent's.
